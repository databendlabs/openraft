//! The single-threaded executor behind [`SimRuntime`](crate::SimRuntime).
//!
//! The `AsyncRuntime` trait is made of static functions (`spawn`, `sleep`, `Instant::now`), so the
//! executor is reached through a thread-local that [`block_on`] installs for the duration of a run.
//!
//! Scheduling is FIFO: a task is polled in the order it became runnable. When nothing is runnable,
//! virtual time jumps to the earliest pending timer; timers sharing a deadline fire in registration
//! order, keyed by `(deadline, seq)`. Every scheduling decision and timer fire is appended to a
//! trace, so two runs can be compared line by line.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::Weak;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::task::Context;
use std::task::Poll;
use std::task::Wake;
use std::task::Waker;

use openraft_rt::OptionalSend;

/// Identifies a task within one runtime. The future passed to `block_on` is task 0.
pub(crate) type TaskId = u64;

const MAIN_TASK: TaskId = 0;

/// Virtual time starts one day in, so code that subtracts a timeout from `now` does not underflow.
pub(crate) const EPOCH_NANOS: u64 = 86_400 * 1_000_000_000;

/// Polls allowed while virtual time stands still before the executor reports a busy loop.
const MAX_POLLS_PER_INSTANT: u64 = 1_000_000;

/// A spawned task, as the executor stores it.
pub(crate) trait TaskFuture: Future<Output = ()> + OptionalSend {}

impl<F> TaskFuture for F where F: Future<Output = ()> + OptionalSend {}

static NEXT_RUNTIME_ID: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static CURRENT: RefCell<Option<Arc<Shared>>> = const { RefCell::new(None) };
}

/// Everything one runtime owns, shared with the wakers and timers of its tasks.
pub(crate) struct Shared {
    /// Distinguishes runtimes on the same thread, so a timer never cancels another runtime's entry.
    pub(crate) id: u64,
    state: Mutex<State>,
}

pub(crate) struct State {
    /// Virtual time in nanoseconds.
    pub(crate) now: u64,
    /// The task being polled; `None` while the executor itself fires timers.
    current: Option<TaskId>,
    next_task: TaskId,
    next_timer_seq: u64,
    run_queue: VecDeque<TaskId>,
    queued: BTreeSet<TaskId>,
    /// `None` while the task is out of the map being polled.
    tasks: BTreeMap<TaskId, Option<Pin<Box<dyn TaskFuture>>>>,
    timers: BTreeMap<(u64, u64), Waker>,
    seed: u64,
    rng_draws: u64,
    polls_at_instant: u64,
    /// Whether events are recorded; formatting every poll is too costly to leave on by default.
    record: bool,
    trace: Vec<String>,
}

impl Shared {
    pub(crate) fn new(seed: u64) -> Arc<Self> {
        Arc::new(Shared {
            id: NEXT_RUNTIME_ID.fetch_add(1, Ordering::Relaxed),
            state: Mutex::new(State {
                now: EPOCH_NANOS,
                current: None,
                next_task: MAIN_TASK + 1,
                next_timer_seq: 0,
                run_queue: VecDeque::new(),
                queued: BTreeSet::new(),
                tasks: BTreeMap::new(),
                timers: BTreeMap::new(),
                seed,
                rng_draws: 0,
                polls_at_instant: 0,
                record: false,
                trace: Vec::new(),
            }),
        })
    }

    /// Locks the state. A panic inside a task never happens under this lock, so poisoning only
    /// follows a panic in the executor itself; the state is still consistent enough to drop.
    pub(crate) fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn set_seed(&self, seed: u64) {
        self.lock().seed = seed;
    }

    pub(crate) fn set_record(&self, record: bool) {
        self.lock().record = record;
    }

    pub(crate) fn take_trace(&self) -> Vec<String> {
        std::mem::take(&mut self.lock().trace)
    }

    /// Drops every remaining task with this runtime installed, so drop code that reads the clock
    /// or cancels timers still works. Tasks spawned while dropping are dropped too.
    pub(crate) fn drop_tasks(self: &Arc<Self>) {
        let _enter = Enter::try_new(self.clone());
        loop {
            let tasks: Vec<_> = {
                let mut st = self.lock();
                st.run_queue.clear();
                st.queued.clear();
                std::mem::take(&mut st.tasks).into_values().flatten().collect()
            };
            if tasks.is_empty() {
                break;
            }
            drop(tasks);
        }
    }

    /// Adds a task and makes it runnable.
    pub(crate) fn spawn(&self, future: Pin<Box<dyn TaskFuture>>) -> TaskId {
        let mut st = self.lock();
        let id = st.next_task;
        st.next_task += 1;
        st.tasks.insert(id, Some(future));
        let by = st.who();
        if st.record {
            st.trace.push(format!("spawn t{id} by {by}"));
        }
        st.enqueue(id);
        id
    }
}

impl State {
    fn who(&self) -> String {
        match self.current {
            Some(id) => format!("t{id}"),
            None => "exec".to_string(),
        }
    }

    fn enqueue(&mut self, id: TaskId) -> bool {
        if self.queued.insert(id) {
            self.run_queue.push_back(id);
            true
        } else {
            false
        }
    }

    fn wake(&mut self, id: TaskId) {
        if id != MAIN_TASK && !self.tasks.contains_key(&id) {
            return;
        }
        if self.enqueue(id) {
            let by = self.who();
            if self.record {
                self.trace.push(format!("wake t{id} by {by}"));
            }
        }
    }

    fn pop_runnable(&mut self) -> Option<TaskId> {
        let id = self.run_queue.pop_front()?;
        self.queued.remove(&id);
        Some(id)
    }

    fn begin_poll(&mut self, id: TaskId) {
        self.current = Some(id);
        self.polls_at_instant += 1;
        if self.polls_at_instant > MAX_POLLS_PER_INSTANT {
            panic!(
                "rt-sim: {} polls at virtual time {}ns without time advancing: a task is busy-looping",
                MAX_POLLS_PER_INSTANT,
                self.now - EPOCH_NANOS
            );
        }
        if self.record {
            self.trace.push(format!("poll t{id} @{}", self.now - EPOCH_NANOS));
        }
    }

    /// Registers a timer and returns its sequence number.
    pub(crate) fn add_timer(&mut self, deadline: u64, waker: Waker) -> u64 {
        let seq = self.next_timer_seq;
        self.next_timer_seq += 1;
        self.timers.insert((deadline, seq), waker);
        let by = self.who();
        if self.record {
            self.trace.push(format!("timer+ ({},{seq}) by {by}", deadline - EPOCH_NANOS));
        }
        seq
    }

    /// Replaces the waker of a registered timer. Returns `false` if the timer is gone.
    pub(crate) fn refresh_timer(&mut self, deadline: u64, seq: u64, waker: &Waker) -> bool {
        match self.timers.get_mut(&(deadline, seq)) {
            Some(registered) => {
                if !registered.will_wake(waker) {
                    *registered = waker.clone();
                }
                true
            }
            None => false,
        }
    }

    /// Removes a timer that has not fired yet.
    pub(crate) fn cancel_timer(&mut self, deadline: u64, seq: u64) {
        if self.timers.remove(&(deadline, seq)).is_some() {
            let by = self.who();
            if self.record {
                self.trace.push(format!("timer- ({},{seq}) by {by}", deadline - EPOCH_NANOS));
            }
        }
    }

    /// A seed for one `thread_rng()` call, derived from the runtime seed and the draw count.
    pub(crate) fn next_rng_seed(&mut self) -> u64 {
        let draw = self.rng_draws;
        self.rng_draws += 1;
        let by = self.who();
        if self.record {
            self.trace.push(format!("rng #{draw} by {by}"));
        }
        splitmix64(self.seed ^ splitmix64(draw))
    }
}

/// SplitMix64, the same mixer `openraft_rt::deterministic_rng` uses.
fn splitmix64(seed: u64) -> u64 {
    let z = seed.wrapping_add(0x9e3779b97f4a7c15);
    let z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    let z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

struct TaskWaker {
    id: TaskId,
    shared: Weak<Shared>,
}

impl Wake for TaskWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        if let Some(shared) = self.shared.upgrade() {
            shared.lock().wake(self.id);
        }
    }
}

fn waker_for(shared: &Arc<Shared>, id: TaskId) -> Waker {
    Waker::from(Arc::new(TaskWaker {
        id,
        shared: Arc::downgrade(shared),
    }))
}

/// The runtime installed on this thread. Panics outside `block_on`.
#[track_caller]
pub(crate) fn current() -> Arc<Shared> {
    try_current().expect("rt-sim: this must be called inside SimRuntime::block_on")
}

pub(crate) fn try_current() -> Option<Arc<Shared>> {
    CURRENT.try_with(|c| c.borrow().clone()).ok().flatten()
}

/// Installs a runtime on this thread and removes it on drop, including during a panic.
struct Enter;

impl Enter {
    fn new(shared: Arc<Shared>) -> Self {
        Self::try_new(shared).expect("rt-sim: nested block_on is not supported")
    }

    /// Installs `shared` unless a runtime is already installed on this thread.
    fn try_new(shared: Arc<Shared>) -> Option<Self> {
        CURRENT
            .try_with(|c| {
                let mut c = c.borrow_mut();
                if c.is_some() {
                    return false;
                }
                *c = Some(shared);
                true
            })
            .unwrap_or(false)
            .then_some(Enter)
    }
}

impl Drop for Enter {
    fn drop(&mut self) {
        let _ = CURRENT.try_with(|c| c.borrow_mut().take());
    }
}

/// Runs `future` to completion, polling spawned tasks and advancing virtual time as needed.
pub(crate) fn block_on<F>(shared: &Arc<Shared>, future: F) -> F::Output
where F: Future {
    let _enter = Enter::new(shared.clone());
    let mut future = std::pin::pin!(future);
    let main_waker = waker_for(shared, MAIN_TASK);

    {
        let mut st = shared.lock();
        st.current = None;
        if st.record {
            st.trace.push("block_on".to_string());
        }
        #[cfg(feature = "futures-reseed")]
        {
            // The shuffle RNG is a thread-local, and every task is polled on this thread, so one
            // reseed makes the branch order of every `select!` in this run a function of the seed.
            let seed = st.seed;
            futures_util::reseed(seed);
            if st.record {
                st.trace.push(format!("reseed select! {seed}"));
            }
        }
        st.enqueue(MAIN_TASK);
    }

    loop {
        let next = shared.lock().pop_runnable();
        match next {
            Some(MAIN_TASK) => {
                shared.lock().begin_poll(MAIN_TASK);
                let polled = future.as_mut().poll(&mut Context::from_waker(&main_waker));
                if let Poll::Ready(output) = polled {
                    let mut st = shared.lock();
                    st.current = None;
                    if st.record {
                        st.trace.push(format!("ready t{MAIN_TASK}"));
                    }
                    return output;
                }
            }
            Some(id) => poll_task(shared, id),
            None => {
                if !fire_next_timers(shared) {
                    let st = shared.lock();
                    panic!(
                        "rt-sim: deadlock at virtual time {}ns: the main future is pending, no task is runnable and \
                         no timer is pending ({} spawned tasks are blocked)",
                        st.now - EPOCH_NANOS,
                        st.tasks.len()
                    );
                }
            }
        }
    }
}

fn poll_task(shared: &Arc<Shared>, id: TaskId) {
    let taken = {
        let mut st = shared.lock();
        match st.tasks.get_mut(&id).and_then(Option::take) {
            Some(task) => {
                st.begin_poll(id);
                Some(task)
            }
            None => None,
        }
    };
    let Some(mut task) = taken else { return };

    let waker = waker_for(shared, id);
    let polled = task.as_mut().poll(&mut Context::from_waker(&waker));

    let mut st = shared.lock();
    st.current = None;
    match polled {
        Poll::Ready(()) => {
            st.tasks.remove(&id);
            if st.record {
                st.trace.push(format!("ready t{id}"));
            }
            drop(st);
            // Dropped outside the lock: the task may own timers whose drop needs it.
            drop(task);
        }
        Poll::Pending => {
            if let Some(slot) = st.tasks.get_mut(&id) {
                *slot = Some(task);
            }
        }
    }
}

/// Jumps virtual time to the earliest pending timer and wakes every timer due at that instant, in
/// `(deadline, seq)` order. Returns `false` if there is no timer.
fn fire_next_timers(shared: &Arc<Shared>) -> bool {
    let wakers = {
        let mut st = shared.lock();
        st.current = None;
        let Some(&(deadline, _)) = st.timers.keys().next() else {
            return false;
        };
        if deadline > st.now {
            if st.record {
                let event = format!("advance {} -> {}", st.now - EPOCH_NANOS, deadline - EPOCH_NANOS);
                st.trace.push(event);
            }
            st.now = deadline;
            st.polls_at_instant = 0;
        }
        let now = st.now;
        let due: Vec<(u64, u64)> = st.timers.range(..=(now, u64::MAX)).map(|(key, _)| *key).collect();
        let mut wakers = Vec::with_capacity(due.len());
        for key in due {
            if let Some(waker) = st.timers.remove(&key) {
                if st.record {
                    st.trace.push(format!("fire ({},{})", key.0 - EPOCH_NANOS, key.1));
                }
                wakers.push(waker);
            }
        }
        wakers
    };
    for waker in wakers {
        waker.wake();
    }
    true
}
