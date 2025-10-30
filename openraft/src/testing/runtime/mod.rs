//! Suite for testing implementations of [`AsyncRuntime`].

#![allow(missing_docs)]

use std::pin::Pin;
use std::pin::pin;
use std::sync::Arc;
use std::task::Poll;

use crate::async_runtime::Mpsc;
use crate::async_runtime::MpscReceiver;
use crate::async_runtime::MpscSender;
use crate::async_runtime::MpscUnboundedWeakSender;
use crate::async_runtime::MpscWeakSender;
use crate::async_runtime::watch::WatchReceiver;
use crate::async_runtime::watch::WatchSender;
use crate::instant::Instant;
use crate::type_config::async_runtime::AsyncRuntime;
use crate::type_config::async_runtime::mpsc_unbounded::MpscUnbounded;
use crate::type_config::async_runtime::mpsc_unbounded::MpscUnboundedReceiver;
use crate::type_config::async_runtime::mpsc_unbounded::MpscUnboundedSender;
use crate::type_config::async_runtime::mpsc_unbounded::TryRecvError;
use crate::type_config::async_runtime::mutex::Mutex;
use crate::type_config::async_runtime::oneshot::Oneshot;
use crate::type_config::async_runtime::oneshot::OneshotSender;
use crate::type_config::async_runtime::watch::Watch;

/// Test suite to ensure a runtime impl works as expected.
///
/// ```rust,ignore
/// struct MyCustomRuntime;
/// impl openraft::AsyncRuntime for MyCustomRuntime { /* omitted */ }
///
/// // Build a runtime
/// let rt = MyCustomRuntime::new();
/// // Run all the tests
/// rt.block_on(Suite::<MyCustomRuntime>::test_all());
/// ```
pub struct Suite<Rt: AsyncRuntime> {
    /// `Rt` needs to be used to make linter happy.
    _marker: std::marker::PhantomData<Rt>,
}

impl<Rt: AsyncRuntime> Suite<Rt> {
    pub async fn test_all() {
        Self::test_spawn_join_handle().await;
        Self::test_sleep().await;
        Self::test_instant().await;
        Self::test_sleep_until().await;
        Self::test_timeout().await;
        Self::test_timeout_at().await;

        Self::test_mpsc_recv_empty().await;
        Self::test_mpsc_recv_channel_closed().await;
        Self::test_mpsc_weak_sender_wont_prevent_channel_close().await;
        Self::test_mpsc_weak_sender_upgrade().await;
        Self::test_mpsc_send().await;

        Self::test_unbounded_mpsc_recv_empty().await;
        Self::test_unbounded_mpsc_recv_channel_closed().await;
        Self::test_unbounded_mpsc_weak_sender_wont_prevent_channel_close().await;
        Self::test_unbounded_mpsc_weak_sender_upgrade().await;
        Self::test_unbounded_mpsc_send().await;

        Self::test_watch_init_value().await;
        Self::test_watch_overwrite_init_value().await;
        Self::test_watch_send_error_no_receiver().await;
        Self::test_watch_send_if_modified().await;
        Self::test_watch_wait_until_ge().await;
        Self::test_watch_wait_until().await;
        Self::test_oneshot_drop_tx().await;
        Self::test_oneshot().await;
        Self::test_mutex().await;
        Self::test_mutex_contention().await;
    }

    pub async fn test_spawn_join_handle() {
        for ret_number in 0..10 {
            let handle = Rt::spawn(async move { ret_number });
            let ret_value = handle.await.unwrap();
            assert_eq!(ret_value, ret_number);
        }
    }

    pub async fn test_sleep() {
        let start_time = std::time::Instant::now();
        let dur_10ms = std::time::Duration::from_millis(10);
        Rt::sleep(dur_10ms).await;
        let elapsed = start_time.elapsed();

        assert!(elapsed >= dur_10ms);
    }

    pub async fn test_instant() {
        let start_time = Rt::Instant::now();
        let dur_10ms = std::time::Duration::from_millis(10);
        Rt::sleep(dur_10ms).await;
        let elapsed = start_time.elapsed();

        assert!(elapsed >= dur_10ms);
    }

    pub async fn test_sleep_until() {
        let start_time = Rt::Instant::now();
        let dur_10ms = std::time::Duration::from_millis(10);
        let end_time = start_time + dur_10ms;
        Rt::sleep_until(end_time).await;
        let elapsed = start_time.elapsed();
        assert!(elapsed >= dur_10ms);
    }

    pub async fn test_timeout() {
        let ret_number = 1;

        // Won't time out
        let dur_10ms = std::time::Duration::from_millis(10);
        let ret_value = Rt::timeout(dur_10ms, async move { ret_number }).await.unwrap();
        assert_eq!(ret_value, ret_number);

        // Will time out
        let dur_1s = std::time::Duration::from_secs(1);
        let timeout_result = Rt::timeout(dur_10ms, async {
            Rt::sleep(dur_1s).await;
            ret_number
        })
        .await;
        assert!(timeout_result.is_err());
    }

    pub async fn test_timeout_at() {
        let ret_number = 1;

        // Won't time out
        let dur_10ms = std::time::Duration::from_millis(10);
        let ddl = Rt::Instant::now() + dur_10ms;
        let ret_value = Rt::timeout_at(ddl, async move { ret_number }).await.unwrap();
        assert_eq!(ret_value, ret_number);

        // Will time out
        let dur_1s = std::time::Duration::from_secs(1);
        let ddl = Rt::Instant::now() + dur_10ms;
        let timeout_result = Rt::timeout_at(ddl, async {
            Rt::sleep(dur_1s).await;
            ret_number
        })
        .await;
        assert!(timeout_result.is_err());
    }

    pub async fn test_mpsc_recv_empty() {
        let (_tx, mut rx) = Rt::Mpsc::channel::<()>(5);
        let recv_err = rx.try_recv().unwrap_err();
        assert!(matches!(recv_err, TryRecvError::Empty));
    }

    pub async fn test_mpsc_recv_channel_closed() {
        let (_, mut rx) = Rt::Mpsc::channel::<()>(5);
        let recv_err = rx.try_recv().unwrap_err();
        assert!(matches!(recv_err, TryRecvError::Disconnected));

        let recv_result = rx.recv().await;
        assert!(recv_result.is_none());
    }

    pub async fn test_mpsc_weak_sender_wont_prevent_channel_close() {
        let (tx, mut rx) = Rt::Mpsc::channel::<()>(5);

        let _weak_tx = tx.downgrade();
        drop(tx);
        let recv_err = rx.try_recv().unwrap_err();
        assert!(matches!(recv_err, TryRecvError::Disconnected));

        let recv_result = rx.recv().await;
        assert!(recv_result.is_none());
    }

    pub async fn test_mpsc_weak_sender_upgrade() {
        let (tx, _rx) = Rt::Mpsc::channel::<()>(5);

        let weak_tx = tx.downgrade();
        let opt_tx = weak_tx.upgrade();
        assert!(opt_tx.is_some());

        drop(tx);
        drop(opt_tx);
        // now there is no Sender instances alive

        let opt_tx = weak_tx.upgrade();
        assert!(opt_tx.is_none());
    }

    pub async fn test_mpsc_send() {
        let (tx, mut rx) = Rt::Mpsc::channel::<usize>(5);
        let tx = Arc::new(tx);

        let n_senders = 10_usize;
        let recv_expected = (0..n_senders).collect::<Vec<_>>();

        for idx in 0..n_senders {
            let tx = tx.clone();
            // no need to wait for senders here, we wait by recv()ing
            let _handle = Rt::spawn(async move {
                tx.send(idx).await.unwrap();
            });
        }

        let mut recv = Vec::with_capacity(n_senders);
        while let Some(recv_number) = rx.recv().await {
            recv.push(recv_number);

            if recv.len() == n_senders {
                break;
            }
        }

        recv.sort();

        assert_eq!(recv_expected, recv);
    }

    pub async fn test_unbounded_mpsc_recv_empty() {
        let (_tx, mut rx) = Rt::MpscUnbounded::channel::<()>();
        let recv_err = rx.try_recv().unwrap_err();
        assert!(matches!(recv_err, TryRecvError::Empty));
    }

    pub async fn test_unbounded_mpsc_recv_channel_closed() {
        let (_, mut rx) = Rt::MpscUnbounded::channel::<()>();
        let recv_err = rx.try_recv().unwrap_err();
        assert!(matches!(recv_err, TryRecvError::Disconnected));

        let recv_result = rx.recv().await;
        assert!(recv_result.is_none());
    }

    pub async fn test_unbounded_mpsc_weak_sender_wont_prevent_channel_close() {
        let (tx, mut rx) = Rt::MpscUnbounded::channel::<()>();

        let _weak_tx = tx.downgrade();
        drop(tx);
        let recv_err = rx.try_recv().unwrap_err();
        assert!(matches!(recv_err, TryRecvError::Disconnected));

        let recv_result = rx.recv().await;
        assert!(recv_result.is_none());
    }

    pub async fn test_unbounded_mpsc_weak_sender_upgrade() {
        let (tx, _rx) = Rt::MpscUnbounded::channel::<()>();

        let weak_tx = tx.downgrade();
        let opt_tx = weak_tx.upgrade();
        assert!(opt_tx.is_some());

        drop(tx);
        drop(opt_tx);
        // now there is no Sender instances alive

        let opt_tx = weak_tx.upgrade();
        assert!(opt_tx.is_none());
    }

    pub async fn test_unbounded_mpsc_send() {
        let (tx, mut rx) = Rt::MpscUnbounded::channel::<usize>();
        let tx = Arc::new(tx);

        let n_senders = 10_usize;
        let recv_expected = (0..n_senders).collect::<Vec<_>>();

        for idx in 0..n_senders {
            let tx = tx.clone();
            // no need to wait for senders here, we wait by recv()ing
            let _handle = Rt::spawn(async move {
                tx.send(idx).unwrap();
            });
        }

        let mut recv = Vec::with_capacity(n_senders);
        while let Some(recv_number) = rx.recv().await {
            recv.push(recv_number);

            if recv.len() == n_senders {
                break;
            }
        }

        recv.sort();

        assert_eq!(recv_expected, recv);
    }

    pub async fn test_watch_init_value() {
        let init_value = 1;
        let (tx, rx) = Rt::Watch::channel(init_value);
        let value_from_rx = rx.borrow_watched();
        assert_eq!(*value_from_rx, init_value);
        let value_from_tx = tx.borrow_watched();
        assert_eq!(*value_from_tx, init_value);
    }

    pub async fn test_watch_overwrite_init_value() {
        let init_value = 1;
        let overwrite = 3;
        assert_ne!(init_value, overwrite);

        let (tx, mut rx) = Rt::Watch::channel(init_value);
        let value_from_rx = rx.borrow_watched();
        let value_from_tx = tx.borrow_watched();
        assert_eq!(*value_from_rx, init_value);
        assert_eq!(*value_from_tx, init_value);
        // drop value so that the immutable ref to `rx`(`tx`) created by
        // `borrow_watched()` can be eliminated, need this because `changed()`
        // will borrows it mutably.
        drop(value_from_rx);
        drop(value_from_tx);

        // macro `pin!` creates a temporary mutable reference to `rx`, move them
        // into a block so that they can be dropped before invoking `.borrow_watched()`,
        // which needs an immutable reference to `rx`.
        {
            let changed_fut = rx.changed();
            let mut pinned_changed_fut = pin!(changed_fut);
            assert!(matches!(poll_in_place(pinned_changed_fut.as_mut()), Poll::Pending));
            tx.send(overwrite).unwrap();
            assert!(matches!(poll_in_place(pinned_changed_fut), Poll::Ready(_)));
        }

        let value_from_rx = rx.borrow_watched();
        let value_from_tx = tx.borrow_watched();
        assert_eq!(*value_from_rx, overwrite);
        assert_eq!(*value_from_tx, overwrite);
    }

    pub async fn test_watch_send_error_no_receiver() {
        let (tx, rx) = Rt::Watch::channel(());
        drop(rx);
        let send_result = tx.send(());
        assert!(send_result.is_err());
    }

    pub async fn test_watch_send_if_modified() {
        let init_value = 0;
        let max_value = 5;
        let n_loop = 10;

        assert!(init_value < max_value);
        assert!(n_loop > max_value);

        let add_one_if_lt_max = |value: &mut i32| {
            if *value < max_value {
                *value += 1;
                true
            } else {
                false
            }
        };

        let (tx, rx) = Rt::Watch::channel(init_value);

        for idx in 0..n_loop {
            let added = tx.send_if_modified(add_one_if_lt_max);

            if idx < max_value {
                assert!(added);
            } else {
                assert!(!added);
            }
        }

        let value_from_rx = rx.borrow_watched();
        assert_eq!(*value_from_rx, max_value);
        let value_from_tx = tx.borrow_watched();
        assert_eq!(*value_from_tx, max_value);
    }

    pub async fn test_watch_wait_until_ge() {
        let init_value = 0;
        let target_value = 5;
        let (tx, mut rx) = Rt::Watch::channel(init_value);

        // Spawn a task that waits for the value to reach target_value
        let handle = Rt::spawn(async move { rx.wait_until_ge(&target_value).await });

        // Send values incrementally
        tx.send(1).unwrap();
        tx.send(3).unwrap();
        // Value should still be waiting since 3 < 5

        tx.send(5).unwrap();
        // Now the wait should complete

        // Verify the returned value is >= target_value
        let final_value = handle.await.unwrap().unwrap();
        assert!(final_value >= target_value);
        assert_eq!(final_value, 5);

        // Test immediate return when value already satisfies condition
        let (tx2, mut rx2) = Rt::Watch::channel(10);
        let returned_value = rx2.wait_until_ge(&5).await.unwrap();
        assert!(returned_value >= 5);
        assert_eq!(returned_value, 10);
        drop(tx2);

        // Test error when sender is dropped before condition is met
        let (tx3, mut rx3) = Rt::Watch::channel(0);
        let handle3 = Rt::spawn(async move { rx3.wait_until_ge(&10).await });
        drop(tx3);
        let result = handle3.await.unwrap();
        assert!(result.is_err());
    }

    pub async fn test_watch_wait_until() {
        // set to an odd number
        let init_value = 1;
        let (tx, mut rx) = Rt::Watch::channel(init_value);

        // Spawn a task that waits for an even value
        let is_even = |v: &i32| v % 2 == 0;
        let handle = Rt::spawn(async move { rx.wait_until(is_even).await });

        // Send odd values
        tx.send(3).unwrap();
        tx.send(5).unwrap();

        // Send an even value to unblock.
        tx.send(6).unwrap();

        let final_value = handle.await.unwrap().unwrap();
        assert_eq!(final_value % 2, 0);
        assert_eq!(final_value, 6);

        // Test immediate return when condition already satisfied
        let (tx2, mut rx2) = Rt::Watch::channel(10);
        let is_greater_than_5 = |v: &i32| *v > 5;
        let returned_value = rx2.wait_until(is_greater_than_5).await.unwrap();
        assert!(returned_value > 5);
        assert_eq!(returned_value, 10);
        drop(tx2);

        // Test error when sender is dropped before condition is met
        let (tx3, mut rx3) = Rt::Watch::channel(0);
        let is_negative = |v: &i32| *v < 0;
        let handle3 = Rt::spawn(async move { rx3.wait_until(is_negative).await });
        drop(tx3);
        let result = handle3.await.unwrap();
        assert!(result.is_err());
    }

    pub async fn test_oneshot_drop_tx() {
        let (tx, rx) = Rt::Oneshot::channel::<()>();
        drop(tx);
        assert!(rx.await.is_err());
    }

    pub async fn test_oneshot() {
        let number_to_send = 1;
        let (tx, rx) = Rt::Oneshot::channel::<i32>();
        tx.send(number_to_send).unwrap();
        let number_received = rx.await.unwrap();

        assert_eq!(number_to_send, number_received);
    }

    pub async fn test_oneshot_send_from_another_task() {
        let number_to_send = 1;
        let (tx, rx) = Rt::Oneshot::channel::<i32>();
        // no need to join the task, this test only works iff the sender task finishes its job
        let _handle = Rt::spawn(async move {
            tx.send(number_to_send).unwrap();
        });
        let number_received = rx.await.unwrap();

        assert_eq!(number_to_send, number_received);
    }

    pub async fn test_mutex_contention() {
        let counter = Arc::new(Rt::Mutex::new(0_u32));
        let n_task = 100;
        let mut handles = Vec::new();

        for _ in 0..n_task {
            let counter = counter.clone();
            let handle = Rt::spawn(async move {
                let mut guard = counter.lock().await;
                *guard += 1;
            });

            handles.push(handle);
        }

        for handle in handles.into_iter() {
            handle.await.unwrap();
        }

        let value = counter.lock().await;
        assert_eq!(*value, n_task);
    }

    pub async fn test_mutex() {
        let lock = Rt::Mutex::new(());
        let guard_fut = lock.lock();
        let pinned_guard_fut = pin!(guard_fut);

        let poll_result = poll_in_place(pinned_guard_fut);
        let guard = match poll_result {
            Poll::Ready(guard) => guard,
            Poll::Pending => panic!("first lock() should succeed"),
        };

        let another_guard_fut = lock.lock();
        let mut pinned_another_guard_fut = pin!(another_guard_fut);
        assert!(matches!(
            poll_in_place(pinned_another_guard_fut.as_mut()),
            Poll::Pending
        ));

        drop(guard);
        assert!(matches!(poll_in_place(pinned_another_guard_fut), Poll::Ready(_)));
    }
}

/// Polls the future, and returns its current state.
fn poll_in_place<F: Future>(fut: Pin<&mut F>) -> Poll<F::Output> {
    let waker = futures::task::noop_waker();
    let mut cx = futures::task::Context::from_waker(&waker);
    fut.poll(&mut cx)
}
