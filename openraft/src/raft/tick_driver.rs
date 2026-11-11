//! One task that wakes many Raft cores when their timer deadlines are due.
//!
//! Each registered core publishes the next instant at which its election,
//! heartbeat, or leadership-transfer check can do work. The driver sleeps
//! until the earliest of those instants and delivers at most one tick per
//! registration until that core consumes it.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use futures_util::FutureExt;
use futures_util::future::Shared;
use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::async_runtime::MpscSender;
use crate::async_runtime::MpscWeakSender;
use crate::async_runtime::OneshotSender;
use crate::async_runtime::Select3;
use crate::async_runtime::Watch;
use crate::async_runtime::WatchReceiver;
use crate::async_runtime::WatchSender;
use crate::base::BoxFuture;
use crate::core::notification::Notification;
use crate::errors::ShutdownError;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::MpscWeakSenderOf;
use crate::type_config::alias::OneshotReceiverOf;
use crate::type_config::alias::OneshotSenderOf;
use crate::type_config::alias::WatchReceiverOf;
use crate::type_config::alias::WatchSenderOf;

/// Drives timer checks for many Raft groups from one task.
///
/// Build a node with [`Raft::build`](crate::Raft::build), pass this driver to
/// [`Raft::use_tick_driver`](crate::Raft::use_tick_driver), then call
/// [`Raft::run`](crate::Raft::run). The node does not spawn an internal ticker.
/// `runtime_config().tick(false)` unschedules that node until ticking is enabled
/// again. [`Trigger::tick`](crate::raft::trigger::Trigger::tick) still enqueues
/// one timer check.
///
/// Shutting down the driver and shutting down a registered core may finish in
/// either order. A core that stops is removed. Dropping the last handle stops
/// the driver task.
#[since(version = "0.10.1")]
pub struct TickDriver<C>
where C: RaftTypeConfig
{
    inner: Arc<Inner<C>>,
}

/// Why [`Raft::use_tick_driver`](crate::Raft::use_tick_driver) refused the driver.
#[since(version = "0.10.1")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum TickSourceError {
    /// [`Raft::run`](crate::Raft::run) has already started the core, or the node
    /// was shut down before `run`.
    #[error("Raft core is no longer waiting to start")]
    AlreadyStarted,
}

/// Which automatic timer source a node uses.
pub(crate) enum TickSource<C>
where C: RaftTypeConfig
{
    /// Spawn the per-member ticker when the core starts.
    Internal,
    /// Register with this driver when the core starts.
    Shared(TickDriver<C>),
    /// The core is registered with a shared driver.
    Registered(TickRegistrationHandle<C>),
}

/// Latest deadline published by one core.
struct DeadlineSlot<C>
where C: RaftTypeConfig
{
    generation: u64,
    deadline: Option<InstantOf<C>>,
}

struct Reg<C>
where C: RaftTypeConfig
{
    slot: Arc<Mutex<DeadlineSlot<C>>>,
    inflight: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
    closed: Arc<AtomicBool>,
    tx: MpscWeakSenderOf<C, Notification<C>>,
    enabled: WatchReceiverOf<C, bool>,
    cancel_rx: WatchReceiverOf<C, bool>,
    seq: AtomicU64,
}

/// Publishes one core's next timer deadline to the shared driver.
pub(crate) struct DeadlinePublisher<C>
where C: RaftTypeConfig
{
    slot: Arc<Mutex<DeadlineSlot<C>>>,
    inflight: Arc<AtomicBool>,
    closed: Arc<AtomicBool>,
    wake: WatchSenderOf<C, u64>,
}

/// Cancels one registration.
///
/// The handle is cloneable so shutdown and `runtime_config().tick` can share it.
#[derive(Clone)]
pub(crate) struct TickRegistrationHandle<C>
where C: RaftTypeConfig
{
    id: u64,
    cancelled: Arc<AtomicBool>,
    cancel_tx: WatchSenderOf<C, bool>,
    regs: Arc<Mutex<BTreeMap<u64, Arc<Reg<C>>>>>,
    wake: WatchSenderOf<C, u64>,
}

struct Inner<C>
where C: RaftTypeConfig
{
    regs: Arc<Mutex<BTreeMap<u64, Arc<Reg<C>>>>>,
    next_id: AtomicU64,
    // `WatchSender` is `Send` but not `Sync`. `Inner` sits in an `Arc`, so the
    // sender is wrapped to make `TickDriver` shareable across Raft tasks.
    wake: Mutex<WatchSenderOf<C, u64>>,
    shutdown: Mutex<Option<OneshotSenderOf<C, ()>>>,
    join_handle: Shared<BoxFuture<'static, Result<(), ShutdownError>>>,
}

impl<C> Clone for TickDriver<C>
where C: RaftTypeConfig
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<C> TickDriver<C>
where C: RaftTypeConfig
{
    /// Start the driver task.
    #[since(version = "0.10.1")]
    pub fn spawn() -> Self {
        let regs = Arc::new(Mutex::new(BTreeMap::new()));
        let (wake, wake_rx) = C::watch_channel(0);
        let (shutdown_tx, shutdown_rx) = C::oneshot();
        let task_regs = regs.clone();
        let task_wake = wake.clone();
        let task = C::spawn(drive::<C>(task_regs, task_wake, wake_rx, shutdown_rx));
        let join_handle = share_join::<C>(task);
        let inner = Inner {
            regs,
            next_id: AtomicU64::new(1),
            wake: Mutex::new(wake),
            shutdown: Mutex::new(Some(shutdown_tx)),
            join_handle,
        };
        Self { inner: Arc::new(inner) }
    }

    /// Stop the driver task and wait for it to finish.
    ///
    /// A second call returns the same result.
    #[since(version = "0.10.1")]
    pub async fn shutdown(&self) -> Result<(), ShutdownError> {
        self.signal_shutdown();
        self.inner.join_handle.clone().await
    }

    pub(crate) fn register(
        &self,
        tx: MpscWeakSenderOf<C, Notification<C>>,
        enabled: WatchReceiverOf<C, bool>,
    ) -> (DeadlinePublisher<C>, TickRegistrationHandle<C>) {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let slot = Arc::new(Mutex::new(DeadlineSlot {
            generation: 0,
            deadline: None,
        }));
        let inflight = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::new(AtomicBool::new(false));
        let closed = Arc::new(AtomicBool::new(false));
        let (cancel_tx, cancel_rx) = C::watch_channel(false);
        let reg = Reg {
            slot: slot.clone(),
            inflight: inflight.clone(),
            cancelled: cancelled.clone(),
            closed: closed.clone(),
            tx,
            enabled,
            cancel_rx,
            seq: AtomicU64::new(0),
        };
        let reg = Arc::new(reg);
        self.inner.regs.lock().unwrap().insert(id, reg);
        let wake = self.inner.wake.lock().unwrap().clone();
        poke(&wake);

        let publisher = DeadlinePublisher {
            slot,
            inflight,
            closed,
            wake: wake.clone(),
        };
        let handle = TickRegistrationHandle {
            id,
            cancelled,
            cancel_tx,
            regs: self.inner.regs.clone(),
            wake,
        };
        (publisher, handle)
    }

    fn signal_shutdown(&self) {
        let sender = {
            let mut guard = self.inner.shutdown.lock().unwrap();
            guard.take()
        };
        if let Some(sender) = sender {
            let send_res = sender.send(());
            tracing::info!("tick driver shutdown signal sent: {send_res:?}");
        }
    }
}

impl<C> Drop for TickDriver<C>
where C: RaftTypeConfig
{
    fn drop(&mut self) {
        // The task does not hold `Inner`. The last handle drops the shutdown
        // sender, which wakes the task. Signal explicitly as well so a handle
        // that is still alive after `shutdown()` does not wait for that drop.
        if Arc::strong_count(&self.inner) == 1 {
            self.signal_shutdown();
        }
    }
}

impl<C> DeadlinePublisher<C>
where C: RaftTypeConfig
{
    /// Replace this core's deadline.
    ///
    /// `None` means the core has no timer work. An unchanged value does not
    /// wake the driver.
    pub(crate) fn publish(&self, deadline: Option<InstantOf<C>>) {
        let mut slot = self.slot.lock().unwrap();
        if slot.deadline == deadline {
            return;
        }
        slot.generation = slot.generation.wrapping_add(1);
        slot.deadline = deadline;
        drop(slot);
        poke(&self.wake);
    }

    /// The core took the outstanding tick. Another due deadline may be sent.
    pub(crate) fn consumed(&self) {
        self.inflight.store(false, Ordering::Release);
        poke(&self.wake);
    }
}

impl<C> Drop for DeadlinePublisher<C>
where C: RaftTypeConfig
{
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
        poke(&self.wake);
    }
}

impl<C> TickRegistrationHandle<C>
where C: RaftTypeConfig
{
    /// Remove this registration. A send that is waiting is cancelled.
    pub(crate) fn cancel(&self) {
        let first = self.cancelled.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_ok();
        if !first {
            return;
        }
        let _ = self.cancel_tx.send(true);
        self.regs.lock().unwrap().remove(&self.id);
        poke(&self.wake);
    }

    /// Wake the driver so it re-reads `enable_tick` and the stored deadline.
    pub(crate) fn poke(&self) {
        poke(&self.wake);
    }
}

fn poke<W>(wake: &impl WatchSender<W, u64>)
where W: Watch {
    wake.send_if_modified(|value| {
        *value = value.wrapping_add(1);
        true
    });
}

fn share_join<C>(
    task: crate::type_config::alias::JoinHandleOf<C, ()>,
) -> Shared<BoxFuture<'static, Result<(), ShutdownError>>>
where C: RaftTypeConfig {
    let future: BoxFuture<'static, _> = Box::pin(async move {
        let result = task.await;
        tracing::info!("tick driver exited: {result:?}");
        if result.is_ok() { Ok(()) } else { Err(ShutdownError) }
    });
    future.shared()
}

async fn drive<C>(
    regs: Arc<Mutex<BTreeMap<u64, Arc<Reg<C>>>>>,
    wake: WatchSenderOf<C, u64>,
    mut wake_rx: WatchReceiverOf<C, u64>,
    mut shutdown_rx: OneshotReceiverOf<C, ()>,
) where
    C: RaftTypeConfig,
{
    // Mark the initial wake counter seen so the first `changed()` waits.
    let _ = wake_rx.borrow_watched();

    loop {
        let queue = rebuild_queue(&regs);
        let earliest = queue.keys().next().map(|(deadline, _id)| *deadline);
        let sleep = sleep_until::<C>(earliest);
        let selected = C::select3(&mut shutdown_rx, wake_rx.changed(), sleep).await;
        match selected {
            Select3::First(_) => {
                tracing::info!("tick driver received shutdown");
                return;
            }
            Select3::Second(Err(_)) => {
                tracing::info!("tick driver wake channel closed");
                return;
            }
            Select3::Second(Ok(())) => {}
            Select3::Third(()) => {
                dispatch_due(&regs, &queue, &wake);
            }
        }
    }
}

fn rebuild_queue<C>(regs: &Mutex<BTreeMap<u64, Arc<Reg<C>>>>) -> BTreeMap<(InstantOf<C>, u64), u64>
where C: RaftTypeConfig {
    let mut queue = BTreeMap::new();
    let mut remove = Vec::new();
    {
        let guard = regs.lock().unwrap();
        for (id, reg) in guard.iter() {
            if reg.cancelled.load(Ordering::Acquire) || reg.closed.load(Ordering::Acquire) {
                remove.push(*id);
                continue;
            }
            let enabled = {
                let value = reg.enabled.borrow_watched();
                *value
            };
            if !enabled || reg.inflight.load(Ordering::Acquire) {
                continue;
            }
            let slot = reg.slot.lock().unwrap();
            let Some(deadline) = slot.deadline else {
                continue;
            };
            queue.insert((deadline, *id), slot.generation);
        }
    }
    if !remove.is_empty() {
        let mut guard = regs.lock().unwrap();
        for id in remove {
            guard.remove(&id);
        }
    }
    queue
}

fn dispatch_due<C>(
    regs: &Mutex<BTreeMap<u64, Arc<Reg<C>>>>,
    queue: &BTreeMap<(InstantOf<C>, u64), u64>,
    wake: &WatchSenderOf<C, u64>,
) where
    C: RaftTypeConfig,
{
    let now = C::now();
    for ((deadline, id), generation) in queue {
        if *deadline > now {
            break;
        }
        let reg = {
            let guard = regs.lock().unwrap();
            guard.get(id).cloned()
        };
        let Some(reg) = reg else {
            continue;
        };
        if !claim(&reg, *generation) {
            continue;
        }
        spawn_send(reg, wake.clone());
    }
}

fn claim<C>(reg: &Reg<C>, generation: u64) -> bool
where C: RaftTypeConfig {
    if reg.cancelled.load(Ordering::Acquire) || reg.closed.load(Ordering::Acquire) {
        return false;
    }
    let claimed = reg.inflight.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_ok();
    if !claimed {
        return false;
    }
    let slot = reg.slot.lock().unwrap();
    let fresh = slot.generation == generation && slot.deadline.is_some_and(|deadline| deadline <= C::now());
    drop(slot);
    if fresh {
        return true;
    }
    reg.inflight.store(false, Ordering::Release);
    false
}

fn spawn_send<C>(reg: Arc<Reg<C>>, wake: WatchSenderOf<C, u64>)
where C: RaftTypeConfig {
    let tx = reg.tx.clone();
    let inflight = Arc::clone(&reg.inflight);
    let closed = Arc::clone(&reg.closed);
    let mut cancel_rx = reg.cancel_rx.clone();
    let previous = reg.seq.fetch_add(1, Ordering::Relaxed);
    let seq = previous.wrapping_add(1);
    let _join = C::spawn(async move {
        let already_cancelled = {
            let value = cancel_rx.borrow_watched();
            *value
        };
        if already_cancelled {
            inflight.store(false, Ordering::Release);
            poke(&wake);
            return;
        }
        let Some(tx) = tx.upgrade() else {
            inflight.store(false, Ordering::Release);
            closed.store(true, Ordering::Release);
            poke(&wake);
            return;
        };
        let send = tx.send(Notification::Tick { i: seq });
        let selected = C::select2(cancel_rx.changed(), send).await;
        match selected {
            crate::async_runtime::Select2::First(_) => {
                inflight.store(false, Ordering::Release);
                poke(&wake);
            }
            crate::async_runtime::Select2::Second(Ok(())) => {}
            crate::async_runtime::Select2::Second(Err(_)) => {
                inflight.store(false, Ordering::Release);
                closed.store(true, Ordering::Release);
                poke(&wake);
            }
        }
    });
}

async fn sleep_until<C>(deadline: Option<InstantOf<C>>)
where C: RaftTypeConfig {
    let Some(deadline) = deadline else {
        std::future::pending::<()>().await;
        return;
    };
    C::sleep_until(deadline).await;
}

#[cfg(test)]
mod tick_driver_test;
