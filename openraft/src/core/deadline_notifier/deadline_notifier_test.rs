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
        let notifier = DeadlineNotifier::<C>::spawn(tx, |i| Notification::Tick { i });

        let late = C::now() + Duration::from_secs(1);
        notifier.set_deadline(Some(late));

        let early = C::now() + Duration::from_millis(10);
        notifier.set_deadline(Some(early));

        let received = C::timeout(Duration::from_secs(1), rx.recv()).await;
        let notification = received.unwrap().unwrap();
        assert!(
            matches!(notification, Notification::Tick { i: 1 }),
            "got: {notification}"
        );

        let next = C::now() + Duration::from_millis(10);
        notifier.set_deadline(Some(next));

        let received = C::timeout(Duration::from_secs(1), rx.recv()).await;
        let notification = received.unwrap().unwrap();
        assert!(
            matches!(notification, Notification::Tick { i: 2 }),
            "got: {notification}"
        );
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
        let (cancel, future) =
            DeadlineNotifier::<C>::run(deadline_rx, tx, |_| Notification::PendingReadDeadlineReached);
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

#[test]
fn deadline_notifier_cancels_a_pending_wait() {
    C::run(async {
        const TIMEOUT: Duration = Duration::from_secs(1);

        for deadline in [None, Some(C::now() + Duration::from_secs(60))] {
            tracing::info!(?deadline, "wait on a deadline that is not reached");
            let (tx, mut rx) = C::mpsc(1);
            let (deadline_tx, deadline_rx) = C::watch_channel(deadline);
            let (cancel, future) = DeadlineNotifier::<C>::run(deadline_rx, tx, |i| Notification::Tick { i });
            let mut future = std::pin::pin!(future);
            let pending = future.as_mut().now_or_never();
            assert_eq!(None, pending);

            tracing::info!(?deadline, "cancel the wait while the deadline watch stays open");
            {
                drop(cancel);
                C::timeout(TIMEOUT, future).await.unwrap();

                let next = rx.try_recv();
                let closed = matches!(next, Err(TryRecvError::Disconnected));
                assert!(
                    closed,
                    "cancellation sends nothing and releases the notification sender"
                );
                drop(deadline_tx);
            }
        }
    });
}
