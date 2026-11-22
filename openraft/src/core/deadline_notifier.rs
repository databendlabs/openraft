use std::future::Future;

use futures_util::FutureExt;

use crate::OptionalSend;
use crate::RaftTypeConfig;
use crate::async_runtime::MpscSender;
use crate::async_runtime::watch::WatchReceiver;
use crate::async_runtime::watch::WatchSender;
use crate::core::DeadlineUpdate;
use crate::core::notification::Notification;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::CancelHandleOf;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::JoinHandleOf;
use crate::type_config::alias::MpscSenderOf;
use crate::type_config::alias::WatchReceiverOf;
use crate::type_config::alias::WatchSenderOf;

/// Wakes `RaftCore` with one notification when a deadline that `RaftCore` sets is reached.
///
/// `RaftCore` uses one instance for the earliest pending linearizable read, one for the instant at
/// which a follower's election may be due, and one for a leader's next heartbeat.
///
/// A deadline moved later does not wake the worker: the worker reads the deadline again when its
/// timer fires, and re-arms. A follower's heartbeats therefore move its election deadline without
/// waking the worker, which wakes once per election timeout instead of once per heartbeat.
///
/// After firing, the worker waits on `changed()` instead of re-arming on the deadline it just
/// reported. That deadline has passed, so the next `set_deadline()` that changes the deadline
/// wakes the worker. For pending reads this cannot lose a wake-up: the `drain_expired` that the
/// notification triggers always moves `earliest_deadline()` to a different value, because every
/// deadline left in the queue exceeds `now` and an emptied queue reports `None`. Watch versioning
/// covers the interleaving where the new deadline is set before the worker starts awaiting.
pub(crate) struct DeadlineNotifier<C>
where C: RaftTypeConfig
{
    deadline_tx: WatchSenderOf<C, Option<InstantOf<C>>>,
    _join_handle: JoinHandleOf<C, ()>,
    _cancel: CancelHandleOf<C>,
}

impl<C> DeadlineNotifier<C>
where C: RaftTypeConfig
{
    /// Spawn a worker that sends `notification(n)` when the deadline set by
    /// [`Self::set_deadline`] is reached for the `n`-th time, counting from 1.
    pub(crate) fn spawn(
        notification_tx: MpscSenderOf<C, Notification<C>>,
        notification: fn(u64) -> Notification<C>,
    ) -> Self {
        let (deadline_tx, deadline_rx) = C::watch_channel(None);
        let (cancel, future) = Self::run(deadline_rx, notification_tx, notification);
        let join_handle = C::spawn(future);

        Self {
            deadline_tx,
            _join_handle: join_handle,
            _cancel: cancel,
        }
    }

    /// Set the deadline to fire at; `None` sends nothing. Return whether the deadline changed and
    /// whether the worker was woken.
    ///
    /// It wakes the worker only if the worker could otherwise sleep past the new deadline: when the
    /// new deadline is earlier than the stored one, or the stored one is `None` or has passed. It
    /// stores a later deadline, or `None`, without a wake, because the worker reads the deadline
    /// again when its timer fires at the stored one.
    ///
    /// The time is read here, inside the update, and the worker reads the time before it reads the
    /// deadline. So if the worker found the stored deadline due, any update that runs after that
    /// read also finds it passed, and wakes the worker. In the other order, a later deadline stored
    /// just after the worker found the old one due could be stored without a wake while the worker
    /// waits on `changed()`, and the worker would never arm it.
    pub(crate) fn set_deadline(&self, deadline: Option<InstantOf<C>>) -> DeadlineUpdate {
        let mut changed = false;
        let woke = self.deadline_tx.send_if_modified(|stored| {
            if *stored == deadline {
                return false;
            }
            changed = true;

            let wake = match (*stored, deadline) {
                (None, _) => true,
                (Some(stored), _) if stored <= C::now() => true,
                (Some(stored), Some(new)) => new < stored,
                (Some(_), None) => false,
            };
            *stored = deadline;
            wake
        });
        DeadlineUpdate { changed, woke }
    }

    fn run(
        deadline_rx: WatchReceiverOf<C, Option<InstantOf<C>>>,
        notification_tx: MpscSenderOf<C, Notification<C>>,
        notification: fn(u64) -> Notification<C>,
    ) -> (CancelHandleOf<C>, impl Future<Output = ()> + OptionalSend) {
        let future = Self::do_run(deadline_rx, notification_tx, notification);
        let (cancel, future) = C::cancellable(future);
        // An `async` block awaiting `future` would reserve its bytes twice: once for the captured
        // variable and once for the moved value it polls (rust-lang/rust#62958). `map()` stores
        // it once.
        let future = future.map(|_finished| ());
        (cancel, future)
    }

    async fn do_run(
        mut deadline_rx: WatchReceiverOf<C, Option<InstantOf<C>>>,
        notification_tx: MpscSenderOf<C, Notification<C>>,
        notification: fn(u64) -> Notification<C>,
    ) {
        let mut reached = 0;
        loop {
            // Read the time before the deadline, as `set_deadline()` requires.
            let now = C::now();
            let deadline = *deadline_rx.borrow_and_update();
            let Some(deadline) = deadline else {
                if deadline_rx.changed().await.is_err() {
                    return;
                }
                continue;
            };

            if deadline > now {
                // When the timer fires, read the deadline again: a later one may have been stored
                // without a wake.
                let wait_result = C::timeout_at(deadline, deadline_rx.changed()).await;
                match wait_result {
                    Ok(Ok(())) | Err(_) => continue,
                    Ok(Err(_)) => return,
                }
            }

            reached += 1;
            let send_result = notification_tx.send(notification(reached)).await;
            if send_result.is_err() {
                return;
            }

            if deadline_rx.changed().await.is_err() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod deadline_notifier_test;
