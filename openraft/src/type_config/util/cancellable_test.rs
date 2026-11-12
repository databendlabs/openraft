use std::time::Duration;

use futures_util::FutureExt;

use crate::AsyncRuntime;
use crate::async_runtime::MpscReceiver;
use crate::async_runtime::MpscSender;
use crate::async_runtime::OneshotSender;
use crate::engine::testing::UTConfig;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::AsyncRuntimeOf;

type C = UTConfig;

const TIMEOUT: Duration = Duration::from_secs(1);

#[test]
fn completion_returns_the_future_output() {
    C::run(async {
        let (mut cancel, future) = C::cancellable(async { 7 });
        let join = C::spawn(future);
        let joined = C::timeout(TIMEOUT, join).await.unwrap();
        let actual = joined.unwrap();
        assert_eq!(Some(7), actual);
        cancel.cancel();
    });
}

#[test]
fn cancellation_before_first_poll_wins_over_ready_work() {
    C::run(async {
        let (tx, rx) = C::oneshot();
        let (mut cancel, future) = C::cancellable(async move { tx.send(7).unwrap() });
        cancel.cancel();
        cancel.cancel();
        let actual = C::timeout(TIMEOUT, future).await.unwrap();
        assert_eq!(None, actual);
        let received = rx.await;
        assert!(received.is_err(), "cancelled work must be dropped without running");
    });
}

#[test]
fn cancellation_while_sleeping() {
    C::run(async {
        let (mut cancel, future) = C::cancellable(async {
            C::sleep(TIMEOUT * 60).await;
            7
        });
        let mut future = std::pin::pin!(future);
        let pending = future.as_mut().now_or_never();
        assert!(pending.is_none());
        cancel.cancel();
        let actual = C::timeout(TIMEOUT, future).await.unwrap();
        assert_eq!(None, actual);
    });
}

#[test]
fn cancellation_during_a_full_channel_send() {
    C::run(async {
        let (tx, mut rx) = C::mpsc(1);
        tx.send(1).await.unwrap();
        let (mut cancel, future) = C::cancellable(async move { tx.send(2).await.unwrap() });
        let mut future = std::pin::pin!(future);
        let pending = future.as_mut().now_or_never();
        assert!(pending.is_none(), "the full channel must block the send");
        cancel.cancel();
        let actual = C::timeout(TIMEOUT, future).await.unwrap();
        assert_eq!(None, actual);
        let first = rx.recv().await;
        assert_eq!(Some(1), first);
        let next = rx.recv().await;
        assert_eq!(None, next, "cancellation must drop the blocked sender");
    });
}

#[test]
fn dropping_handle_cancels_spawned_work() {
    C::run(async {
        let (tx, rx) = C::oneshot();
        let (cancel, future) = C::cancellable(async move {
            std::future::pending::<()>().await;
            tx.send(7).unwrap();
        });
        let join = C::spawn(future);
        drop(cancel);
        let joined = C::timeout(TIMEOUT, join).await.unwrap();
        let actual = joined.unwrap();
        assert_eq!(None, actual);
        let received = rx.await;
        assert!(
            received.is_err(),
            "dropping the handle must release the work's captures"
        );
    });
}

#[test]
fn completion_after_pending() {
    C::run(async {
        let (ready, completed) = C::oneshot::<u64>();
        let (_cancel, future) = C::cancellable(async move { completed.await.unwrap() });
        let mut future = std::pin::pin!(future);
        let pending = future.as_mut().now_or_never();
        assert!(pending.is_none());
        ready.send(7).unwrap();
        let actual = future.await;
        assert_eq!(Some(7), actual);
    });
}

#[test]
fn panic_reaches_runtime_join() {
    C::run(async {
        let (_cancel, future) = C::cancellable(async { panic!("injected cancellable future panic") });
        let join = C::spawn(future);
        let result = C::timeout(TIMEOUT, join).await.unwrap();
        let error = result.unwrap_err();
        let panicked = AsyncRuntimeOf::<C>::is_panic(&error);
        assert!(panicked);
    });
}

#[test]
fn accepts_a_borrowed_future() {
    C::run(async {
        let mut value = 0;
        let (_cancel, future) = C::cancellable(async {
            value = 7;
            value
        });
        let actual = future.await;
        assert_eq!(Some(7), actual);
        assert_eq!(7, value);
    });
}

#[cfg(feature = "single-threaded")]
#[test]
fn accepts_a_local_future_and_output() {
    use std::rc::Rc;

    C::run(async {
        let value = Rc::new(7);
        let (_cancel, future) = C::cancellable(async move { value });
        let join = C::spawn(future);
        let joined = C::timeout(TIMEOUT, join).await.unwrap();
        let actual = joined.unwrap().unwrap();
        let value = *actual;
        assert_eq!(7, value);
    });
}

#[test]
fn by_preserves_the_work_output() {
    C::run(async {
        let future = C::cancellable_by(async { None::<u64> }, std::future::pending::<()>());
        let actual = future.await;
        assert_eq!(Some(None), actual);
    });
}

#[test]
fn by_prioritizes_a_ready_cancel_signal() {
    C::run(async {
        let future = C::cancellable_by(async { 7 }, async { "cancel" });
        let actual = future.await;
        assert_eq!(None, actual);
    });
}

#[test]
fn by_signal_wakes_the_waiter() {
    C::run(async {
        let (started, running) = C::oneshot();
        let work = async move {
            started.send(()).unwrap();
            std::future::pending::<u64>().await
        };
        let (cancel, signal) = C::oneshot::<()>();
        let future = C::cancellable_by(work, signal);
        let join = C::spawn(future);
        running.await.unwrap();
        cancel.send(()).unwrap();
        let joined = C::timeout(TIMEOUT, join).await.unwrap();
        let actual = joined.unwrap();
        assert_eq!(None, actual);
    });
}
