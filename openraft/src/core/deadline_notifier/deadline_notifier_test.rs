use std::pin::pin;
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::future::Either;
use futures_util::future::select;

use super::*;
use crate::async_runtime::MpscReceiver;
use crate::async_runtime::mpsc::TryRecvError;
use crate::engine::testing::UTConfig;

type C = UTConfig;

#[test]
fn deadline_notifier_uses_latest_deadline_and_rearms() {
    C::run(async {
        let (tx, mut rx) = C::mpsc(4);

        tracing::info!("let the worker wait on a far deadline");
        let far = C::now() + Duration::from_secs(60);
        let (deadline_tx, deadline_rx) = C::watch_channel(Some(far));
        let (_cancel, future) = DeadlineNotifier::<C>::run(deadline_rx, tx, |i| Notification::Tick { i });
        let mut future = pin!(future);
        let waiting = future.as_mut().now_or_never().is_none();
        assert!(waiting, "the worker waits for the far deadline");

        tracing::info!("a deadline that is already due interrupts the wait at once, and reports once");
        {
            let due = C::now();
            deadline_tx.send(Some(due)).unwrap();
            let waiting = future.as_mut().now_or_never().is_none();
            assert!(
                waiting,
                "the worker reports the due deadline, then waits for the next change"
            );

            let notification = rx.try_recv().unwrap();
            assert!(
                matches!(notification, Notification::Tick { i: 1 }),
                "got: {notification}"
            );

            let waiting = future.as_mut().now_or_never().is_none();
            assert!(waiting, "the worker still waits for the next change");
            let next = rx.try_recv();
            assert!(matches!(next, Err(TryRecvError::Empty)), "reported more than once");
        }

        tracing::info!("a new deadline rearms the worker");
        {
            let next = C::now() + Duration::from_millis(10);
            deadline_tx.send(Some(next)).unwrap();

            let received = pin!(rx.recv());
            let first = C::timeout(Duration::from_secs(1), select(future.as_mut(), received)).await;
            let Either::Right((notification, _worker)) = first.unwrap() else {
                panic!("the worker returned while its deadline watch is open");
            };
            let notification = notification.unwrap();
            assert!(
                matches!(notification, Notification::Tick { i: 2 }),
                "got: {notification}"
            );
        }
    });
}

#[test]
fn deadline_notifier_wakes_the_worker_only_when_it_could_sleep_past_the_deadline() {
    C::run(async {
        let (tx, _rx) = C::mpsc(4);
        let notifier = DeadlineNotifier::<C>::spawn(tx, |i| Notification::Tick { i });
        let mut watcher = notifier.deadline_tx.subscribe();
        let mut woken = move || watcher.changed().now_or_never().is_some();

        let now = C::now();
        let in_secs = |secs| Some(now + Duration::from_secs(secs));
        let changed_with_wake = DeadlineUpdate {
            changed: true,
            woke: true,
        };
        let changed_without_wake = DeadlineUpdate {
            changed: true,
            woke: false,
        };

        tracing::info!("a first deadline, and an earlier one, wake the worker");
        {
            assert_eq!(changed_with_wake, notifier.set_deadline(in_secs(60)));
            assert!(woken());
            assert_eq!(changed_with_wake, notifier.set_deadline(in_secs(30)));
            assert!(woken());
        }

        tracing::info!("a later deadline, or none, is stored without a wake");
        {
            assert_eq!(changed_without_wake, notifier.set_deadline(in_secs(120)));
            assert!(!woken());
            assert_eq!(in_secs(120), *notifier.deadline_tx.borrow_watched());
            assert_eq!(changed_without_wake, notifier.set_deadline(None));
            assert!(!woken());
        }

        tracing::info!("once the stored deadline has passed, even a later deadline wakes the worker");
        {
            let passed = Some(now - Duration::from_secs(1));
            assert_eq!(changed_with_wake, notifier.set_deadline(passed));
            assert!(woken());
            assert_eq!(changed_with_wake, notifier.set_deadline(in_secs(120)));
            assert!(woken());
        }

        tracing::info!("an unchanged deadline changes nothing");
        {
            let unchanged = DeadlineUpdate {
                changed: false,
                woke: false,
            };
            assert_eq!(unchanged, notifier.set_deadline(in_secs(120)));
            assert!(!woken());
        }
    });
}

#[test]
fn deadline_notifier_reads_a_later_deadline_when_its_timer_fires() {
    C::run(async {
        let (tx, mut rx) = C::mpsc(4);
        let notifier = DeadlineNotifier::<C>::spawn(tx, |i| Notification::Tick { i });

        tracing::info!("move the deadline later while the worker sleeps until the first one");
        let first = C::now() + Duration::from_millis(300);
        let later = first + Duration::from_millis(300);
        {
            notifier.set_deadline(Some(first));
            C::sleep(Duration::from_millis(100)).await;
            notifier.set_deadline(Some(later));
        }

        tracing::info!("the worker reports only the later deadline, once");
        {
            let received = C::timeout(Duration::from_secs(2), rx.recv()).await;
            let notification = received.unwrap().unwrap();
            assert!(C::now() >= later, "reported before the later deadline");
            assert!(
                matches!(notification, Notification::Tick { i: 1 }),
                "got: {notification}"
            );

            C::sleep(Duration::from_millis(100)).await;
            let next = rx.try_recv();
            assert!(matches!(next, Err(TryRecvError::Empty)), "reported more than once");
        }
    });
}

#[test]
fn deadline_notifier_reports_nothing_once_the_deadline_is_cleared() {
    C::run(async {
        let (tx, mut rx) = C::mpsc(4);
        let notifier = DeadlineNotifier::<C>::spawn(tx, |i| Notification::Tick { i });

        tracing::info!("clear the deadline while the worker sleeps until it");
        let deadline = C::now() + Duration::from_millis(300);
        {
            notifier.set_deadline(Some(deadline));
            C::sleep(Duration::from_millis(100)).await;
            notifier.set_deadline(None);
        }

        tracing::info!("the worker wakes at the cleared deadline and reports nothing");
        {
            C::sleep_until(deadline + Duration::from_millis(200)).await;
            let next = rx.try_recv();
            assert!(
                matches!(next, Err(TryRecvError::Empty)),
                "a cleared deadline was reported"
            );
        }
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
