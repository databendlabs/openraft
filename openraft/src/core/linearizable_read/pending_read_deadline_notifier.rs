use std::future::Future;

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

/// Wakes `RaftCore` when the earliest pending linearizable read reaches its deadline.
///
/// After firing, the worker waits on `changed()` instead of re-arming on the deadline it just
/// reported, which cannot lose a wake-up: the `drain_expired` that the notification triggers always
/// moves `earliest_deadline()` to a different value, because every deadline left in the queue
/// exceeds `now` and an emptied queue reports `None`. `set_deadline` therefore always observes a
/// change and re-arms the worker. Watch versioning covers the interleaving where the new deadline
/// is set before the worker starts awaiting.
pub(crate) struct PendingReadDeadlineNotifier<C>
where C: RaftTypeConfig
{
    deadline_tx: WatchSenderOf<C, Option<InstantOf<C>>>,
    _join_handle: JoinHandleOf<C, ()>,
    _cancel: CancelHandleOf<C>,
}

impl<C> PendingReadDeadlineNotifier<C>
where C: RaftTypeConfig
{
    pub(crate) fn spawn(notification_tx: MpscSenderOf<C, Notification<C>>) -> Self {
        let (deadline_tx, deadline_rx) = C::watch_channel(None);
        let (cancel, future) = Self::run(deadline_rx, notification_tx);
        let join_handle = C::spawn(future);

        Self {
            deadline_tx,
            _join_handle: join_handle,
            _cancel: cancel,
        }
    }

    pub(crate) fn set_deadline(&self, deadline: Option<InstantOf<C>>) {
        self.deadline_tx.send_if_different(deadline);
    }

    fn run(
        deadline_rx: WatchReceiverOf<C, Option<InstantOf<C>>>,
        notification_tx: MpscSenderOf<C, Notification<C>>,
    ) -> (CancelHandleOf<C>, impl Future<Output = ()> + OptionalSend) {
        let future = Self::do_run(deadline_rx, notification_tx);
        let (cancel, future) = C::cancellable(future);
        let future = async move {
            let _ = future.await;
        };
        (cancel, future)
    }

    async fn do_run(
        mut deadline_rx: WatchReceiverOf<C, Option<InstantOf<C>>>,
        notification_tx: MpscSenderOf<C, Notification<C>>,
    ) {
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

            let send_result = notification_tx.send(Notification::PendingReadDeadlineReached).await;
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
mod tests {
    use std::time::Duration;

    use futures_util::FutureExt;

    use super::*;
    use crate::async_runtime::MpscReceiver;
    use crate::async_runtime::mpsc::TryRecvError;
    use crate::engine::testing::UTConfig;

    type C = UTConfig;

    #[test]
    fn deadline_notifier_uses_latest_deadline_and_rearms() {
        C::run(async {
            let (tx, mut rx) = C::mpsc(4);
            let notifier = PendingReadDeadlineNotifier::<C>::spawn(tx);

            let late = C::now() + Duration::from_secs(1);
            notifier.set_deadline(Some(late));

            let early = C::now() + Duration::from_millis(10);
            notifier.set_deadline(Some(early));

            let received = C::timeout(Duration::from_secs(1), rx.recv()).await;
            let notification = received.unwrap().unwrap();
            assert!(matches!(notification, Notification::PendingReadDeadlineReached));

            let next = C::now() + Duration::from_millis(10);
            notifier.set_deadline(Some(next));

            let received = C::timeout(Duration::from_secs(1), rx.recv()).await;
            let notification = received.unwrap().unwrap();
            assert!(matches!(notification, Notification::PendingReadDeadlineReached));
        });
    }

    #[test]
    fn deadline_notifier_cancels_a_blocked_send() {
        C::run(async {
            const TIMEOUT: Duration = Duration::from_secs(1);

            tracing::info!("fill the notification channel before the deadline expires");
            let (tx, mut rx) = C::mpsc(1);
            tx.send(Notification::Tick { i: 1 }).await.unwrap();
            let deadline = C::now() - TIMEOUT;
            let (deadline_tx, deadline_rx) = C::watch_channel(Some(deadline));
            let (cancel, future) = PendingReadDeadlineNotifier::<C>::run(deadline_rx, tx);
            let mut future = std::pin::pin!(future);
            let pending = future.as_mut().now_or_never();
            assert_eq!(None, pending);

            tracing::info!("cancel the blocked send while keeping notification capacity occupied");
            {
                drop(deadline_tx);
                let pending = future.as_mut().now_or_never();
                assert_eq!(None, pending, "closing the deadline watch cannot interrupt a send");
                drop(cancel);
                C::timeout(TIMEOUT, future).await.unwrap();

                let first = rx.try_recv().unwrap();
                let sentinel = matches!(first, Notification::Tick { i: 1 });
                assert!(sentinel, "the blocked deadline notification must be discarded");
                let next = rx.try_recv();
                let closed = matches!(next, Err(TryRecvError::Disconnected));
                assert!(closed, "cancellation must release the notification sender");
            }
        });
    }
}
