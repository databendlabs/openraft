use std::future::Future;

use futures_util::FutureExt;

use crate::OptionalSend;
use crate::RaftTypeConfig;
use crate::async_runtime::MpscSender;
use crate::async_runtime::watch::WatchReceiver;
use crate::async_runtime::watch::WatchSender;
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
/// After firing, the worker waits on `changed()` instead of re-arming on the deadline it just
/// reported. For pending reads this cannot lose a wake-up: the `drain_expired` that the
/// notification triggers always moves `earliest_deadline()` to a different value, because every
/// deadline left in the queue exceeds `now` and an emptied queue reports `None`. `set_deadline`
/// therefore always observes a change and re-arms the worker. Watch versioning covers the
/// interleaving where the new deadline is set before the worker starts awaiting.
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

    /// Set the deadline to fire at; `None` sends nothing. Return whether the deadline changed.
    pub(crate) fn set_deadline(&self, deadline: Option<InstantOf<C>>) -> bool {
        self.deadline_tx.send_if_different(deadline)
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
            let deadline = *deadline_rx.borrow_and_update();
            let Some(deadline) = deadline else {
                if deadline_rx.changed().await.is_err() {
                    return;
                }
                continue;
            };

            let wait_result = C::timeout_at(deadline, deadline_rx.changed()).await;
            match wait_result {
                Ok(Ok(())) => continue,
                Ok(Err(_)) => return,
                Err(_) => {}
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
