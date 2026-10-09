//! The executor state: virtual time, tasks, the run queue, timers and the trace.

use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::pin::Pin;
use std::task::Waker;

use crate::executor::EPOCH_NANOS;
use crate::executor::MAIN_TASK;
use crate::executor::TaskId;
use crate::executor::task_future::TaskFuture;

/// Polls allowed while virtual time stands still before the executor reports a busy loop.
const MAX_POLLS_PER_INSTANT: u64 = 1_000_000;

pub(crate) struct State {
    /// Virtual time in nanoseconds.
    pub(crate) now: u64,
    /// The task being polled; `None` while the executor itself fires timers.
    pub(super) current: Option<TaskId>,
    pub(super) next_task_id: TaskId,
    next_timer_seq: u64,

    pub(super) run_queue: VecDeque<TaskId>,

    /// `None` while the task is out of the map being polled.
    pub(super) tasks: BTreeMap<TaskId, Option<Pin<Box<dyn TaskFuture>>>>,

    pub(super) timers: BTreeMap<(u64, u64), Waker>,

    pub(super) seed: u64,
    rng_draws: u64,

    pub(super) polls_at_instant: u64,
    /// Whether events are recorded; formatting every poll is too costly to leave on by default.
    pub(super) record: bool,
    pub(super) trace: Vec<String>,
}

impl State {
    pub(super) fn new(seed: u64) -> Self {
        State {
            now: EPOCH_NANOS,
            current: None,
            next_task_id: MAIN_TASK + 1,
            next_timer_seq: 0,
            run_queue: VecDeque::new(),
            tasks: BTreeMap::new(),
            timers: BTreeMap::new(),
            seed,
            rng_draws: 0,
            polls_at_instant: 0,
            record: false,
            trace: Vec::new(),
        }
    }

    /// Appends an event to the trace. The event is only formatted while recording.
    pub(super) fn trace(&mut self, event: impl FnOnce(&Self) -> String) {
        if self.record {
            let event = event(self);
            self.trace.push(event);
        }
    }

    pub(super) fn who(&self) -> String {
        match self.current {
            Some(id) => format!("t{id}"),
            None => "exec".to_string(),
        }
    }

    pub(super) fn enqueue(&mut self, id: TaskId) -> bool {
        if !self.run_queue.contains(&id) {
            self.run_queue.push_back(id);
            true
        } else {
            false
        }
    }

    pub(super) fn wake(&mut self, id: TaskId) {
        // `MAIN_TASK` is never in `tasks`, because `block_on` holds the main future. Any other id
        // is missing when a waker outlives its task. For example, `drop_tasks` takes every task out
        // of `tasks` before dropping them, and dropping one task wakes another by closing a channel
        // that the other task waits on. Ignoring such a wake keeps a task that no longer exists out
        // of the run queue and the trace.
        if id != MAIN_TASK && !self.tasks.contains_key(&id) {
            return;
        }
        if self.enqueue(id) {
            self.trace(|st| format!("wake t{id} by {}", st.who()));
        }
    }

    pub(super) fn pop_runnable(&mut self) -> Option<TaskId> {
        self.run_queue.pop_front()
    }

    pub(super) fn begin_poll(&mut self, id: TaskId) {
        self.current = Some(id);
        self.polls_at_instant += 1;
        if self.polls_at_instant > MAX_POLLS_PER_INSTANT {
            panic!(
                "rt-sim: {} polls at virtual time {}ns without time advancing: a task is busy-looping",
                MAX_POLLS_PER_INSTANT,
                self.now - EPOCH_NANOS
            );
        }
        self.trace(|st| format!("poll t{id} @{}", st.now - EPOCH_NANOS));
    }

    /// Returns the sequence number of a new timer. Timers sharing a deadline fire in this order.
    pub(crate) fn new_timer_seq(&mut self) -> u64 {
        let seq = self.next_timer_seq;
        self.next_timer_seq += 1;
        seq
    }

    /// Makes the timer `(deadline, seq)` wake `waker`, and adds the timer if it is not pending.
    pub(crate) fn set_timer(&mut self, deadline: u64, seq: u64, waker: &Waker) {
        let replaced = self.timers.insert((deadline, seq), waker.clone());
        if replaced.is_none() {
            self.trace(|st| format!("timer+ ({},{seq}) by {}", deadline - EPOCH_NANOS, st.who()));
        }
    }

    /// Removes a timer that has not fired yet.
    pub(crate) fn cancel_timer(&mut self, deadline: u64, seq: u64) {
        if self.timers.remove(&(deadline, seq)).is_some() {
            self.trace(|st| format!("timer- ({},{seq}) by {}", deadline - EPOCH_NANOS, st.who()));
        }
    }

    /// A seed for one `thread_rng()` call, derived from the runtime seed and the draw count.
    pub(crate) fn next_rng_seed(&mut self) -> u64 {
        let draw = self.rng_draws;
        self.rng_draws += 1;
        self.trace(|st| format!("rng #{draw} by {}", st.who()));
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
