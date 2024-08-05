//! Suite for testing implementations of [`AsyncRuntime`].

use std::sync::Arc;

use crate::async_runtime::watch::WatchReceiver;
use crate::async_runtime::watch::WatchSender;
use crate::async_runtime::MpscUnboundedWeakSender;
use crate::instant::Instant;
use crate::type_config::async_runtime::mpsc_unbounded::MpscUnbounded;
use crate::type_config::async_runtime::mpsc_unbounded::MpscUnboundedReceiver;
use crate::type_config::async_runtime::mpsc_unbounded::MpscUnboundedSender;
use crate::type_config::async_runtime::mpsc_unbounded::TryRecvError;
use crate::type_config::async_runtime::mutex::Mutex;
use crate::type_config::async_runtime::oneshot::Oneshot;
use crate::type_config::async_runtime::oneshot::OneshotSender;
use crate::type_config::async_runtime::watch::Watch;
use crate::type_config::async_runtime::AsyncRuntime;

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
        Self::test_unbounded_mpsc_recv_empty().await;
        Self::test_unbounded_mpsc_recv_channel_closed().await;
        Self::test_unbounded_mpsc_weak_sender_wont_prevent_channel_close().await;
        Self::test_unbounded_mpsc_weak_sender_upgrade().await;
        Self::test_unbounded_mpsc_send().await;
        Self::test_watch_init_value().await;
        Self::test_watch_overwrite_init_value().await;
        Self::test_watch_send_error_no_receiver().await;
        Self::test_watch_send_if_modified().await;
        Self::test_oneshot_drop_tx().await;
        Self::test_oneshot().await;
        Self::test_mutex().await;
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
        let dur_100ms = std::time::Duration::from_millis(100);
        Rt::sleep(dur_100ms).await;
        let elapsed = start_time.elapsed();

        assert!(elapsed > dur_100ms);
    }

    pub async fn test_instant() {
        let start_time = Rt::Instant::now();
        let dur_100ms = std::time::Duration::from_millis(100);
        Rt::sleep(dur_100ms).await;
        let elapsed = start_time.elapsed();

        assert!(elapsed > dur_100ms);
    }

    pub async fn test_sleep_until() {
        let start_time = Rt::Instant::now();
        let dur_100ms = std::time::Duration::from_millis(100);
        let end_time = start_time + dur_100ms;
        Rt::sleep_until(end_time).await;
        let elapsed = start_time.elapsed();
        assert!(elapsed > dur_100ms);
    }

    pub async fn test_timeout() {
        let ret_number = 1;

        // Won't time out
        let dur_100ms = std::time::Duration::from_millis(100);
        let ret_value = Rt::timeout(dur_100ms, async move { ret_number }).await.unwrap();
        assert_eq!(ret_value, ret_number);

        // Will time out
        let dur_150ms = std::time::Duration::from_millis(150);
        let timeout_result = Rt::timeout(dur_100ms, async {
            Rt::sleep(dur_150ms).await;
            ret_number
        })
        .await;
        assert!(timeout_result.is_err());
    }

    pub async fn test_timeout_at() {
        let ret_number = 1;

        // Won't time out
        let dur_100ms = std::time::Duration::from_millis(100);
        let ddl = Rt::Instant::now() + dur_100ms;
        let ret_value = Rt::timeout_at(ddl, async move { ret_number }).await.unwrap();
        assert_eq!(ret_value, ret_number);

        // Will time out
        let dur_150ms = std::time::Duration::from_millis(150);
        let ddl = Rt::Instant::now() + dur_100ms;
        let timeout_result = Rt::timeout_at(ddl, async {
            Rt::sleep(dur_150ms).await;
            ret_number
        })
        .await;
        assert!(timeout_result.is_err());
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
            let tx = Arc::clone(&tx);
            // no need to wait for senders here, we wait by recv()ing
            let _ = Rt::spawn(async move {
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

        tx.send(overwrite).unwrap();
        rx.changed().await.unwrap();
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
        let _ = Rt::spawn(async move {
            tx.send(number_to_send).unwrap();
        });
        let number_received = rx.await.unwrap();

        assert_eq!(number_to_send, number_received);
    }

    pub async fn test_mutex() {
        let counter = Arc::new(Rt::Mutex::new(0_u32));
        let n_task = 100;
        let mut handles = Vec::new();

        for _ in 0..n_task {
            let counter = Arc::clone(&counter);
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
}
