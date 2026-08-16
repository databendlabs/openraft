use crate::RaftTypeConfig;
use crate::async_runtime::MpscSender;
use crate::async_runtime::watch::WatchReceiver;
use crate::async_runtime::watch::WatchSender;
use crate::core::notification::Notification;
use crate::type_config::TypeConfigExt;
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
}

impl<C> PendingReadDeadlineNotifier<C>
where C: RaftTypeConfig
{
    pub(crate) fn spawn(notification_tx: MpscSenderOf<C, Notification<C>>) -> Self {
        let (deadline_tx, deadline_rx) = C::watch_channel(None);
        let join_handle = C::spawn(Self::run(deadline_rx, notification_tx));

        Self {
            deadline_tx,
            _join_handle: join_handle,
        }
    }

    pub(crate) fn set_deadline(&self, deadline: Option<InstantOf<C>>) {
        self.deadline_tx.send_if_different(deadline);
    }

    async fn run(
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

    use super::*;
    use crate::async_runtime::MpscReceiver;
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
}
