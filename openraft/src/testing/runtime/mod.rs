//! Suite for testing implementations of [`AsyncRuntime`].

#![allow(missing_docs)]

use std::pin::Pin;
use std::pin::pin;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

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
        Self::test_thread_rng().await;
        Self::test_sleep().await;
        Self::test_instant().await;
        Self::test_instant_arithmetic().await;
        Self::test_instant_sub_instant().await;
        Self::test_instant_saturating_duration_since().await;
        Self::test_instant_ord().await;
        Self::test_sleep_until().await;
        Self::test_timeout().await;
        Self::test_timeout_at().await;

        Self::test_mpsc_recv_empty().await;
        Self::test_mpsc_recv_channel_closed().await;
        Self::test_mpsc_weak_sender_wont_prevent_channel_close().await;
        Self::test_mpsc_weak_sender_upgrade().await;
        Self::test_mpsc_send().await;
        Self::test_mpsc_send_to_closed_channel().await;
        Self::test_mpsc_backpressure().await;

        Self::test_unbounded_mpsc_recv_empty().await;
        Self::test_unbounded_mpsc_recv_channel_closed().await;
        Self::test_unbounded_mpsc_weak_sender_wont_prevent_channel_close().await;
        Self::test_unbounded_mpsc_weak_sender_upgrade().await;
        Self::test_unbounded_mpsc_send().await;
        Self::test_unbounded_mpsc_send_to_closed_channel().await;

        Self::test_watch_init_value().await;
        Self::test_watch_overwrite_init_value().await;
        Self::test_watch_send_error_no_receiver().await;
        Self::test_watch_send_if_modified().await;
        Self::test_watch_wait_until_ge().await;
        Self::test_watch_wait_until().await;
        Self::test_watch_changed_marks_as_seen().await;
        Self::test_watch_changed_returns_immediately_when_unseen().await;
        Self::test_watch_multiple_borrow_then_changed().await;
        Self::test_watch_wait_loop_pattern().await;
        Self::test_watch_multiple_receivers().await;
        Self::test_oneshot_drop_tx().await;
        Self::test_oneshot().await;
        Self::test_oneshot_send_from_another_task().await;
        Self::test_oneshot_send_to_dropped_rx().await;
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

    /// Test `thread_rng()` returns a working random number generator.
    pub async fn test_thread_rng() {
        use rand::Rng;

        let mut rng = Rt::thread_rng();

        // Generate some random numbers to verify the RNG works
        let r1: u32 = rng.random();
        let r2: u32 = rng.random();
        let r3: u32 = rng.random();

        // While theoretically possible for all to be equal, it's astronomically unlikely
        // This just verifies the RNG is functioning and producing values
        let all_same = r1 == r2 && r2 == r3;
        assert!(
            !all_same || r1 != 0,
            "RNG should produce varying values (got {}, {}, {})",
            r1,
            r2,
            r3
        );

        // Test range generation
        for _ in 0..100 {
            let value: u32 = rng.random_range(0..100);
            assert!(value < 100, "random_range should respect upper bound");
        }

        // Test bool generation works
        let _: bool = rng.random();
    }

    pub async fn test_sleep() {
        let start_time = std::time::Instant::now();
        let dur_10ms = Duration::from_millis(10);
        Rt::sleep(dur_10ms).await;
        let elapsed = start_time.elapsed();

        assert!(elapsed >= dur_10ms);
    }

    pub async fn test_instant() {
        let start_time = Rt::Instant::now();
        let dur_10ms = Duration::from_millis(10);
        Rt::sleep(dur_10ms).await;
        let elapsed = start_time.elapsed();

        assert!(elapsed >= dur_10ms);
    }

    /// Test `Instant + Duration` and `Instant - Duration` arithmetic.
    pub async fn test_instant_arithmetic() {
        let dur_100ms = Duration::from_millis(100);
        let dur_50ms = Duration::from_millis(50);

        let now = Rt::Instant::now();

        // Test Add<Duration>
        let later = now + dur_100ms;
        assert!(later > now);

        // Test Sub<Duration>
        let earlier = later - dur_50ms;
        assert!(earlier > now);
        assert!(earlier < later);

        // Test AddAssign<Duration>
        let mut t = now;
        t += dur_100ms;
        assert_eq!(t, later);

        // Test SubAssign<Duration>
        let mut t2 = later;
        t2 -= dur_50ms;
        assert_eq!(t2, earlier);
    }

    /// Test `Instant - Instant` returns `Duration`.
    pub async fn test_instant_sub_instant() {
        let dur_50ms = Duration::from_millis(50);

        let t1 = Rt::Instant::now();
        Rt::sleep(dur_50ms).await;
        let t2 = Rt::Instant::now();

        // t2 - t1 should be approximately 50ms (at least 50ms)
        let diff = t2 - t1;
        assert!(diff >= dur_50ms);
        // Should be less than 200ms (reasonable upper bound)
        assert!(diff < Duration::from_millis(200));
    }

    /// Test `saturating_duration_since` returns zero when `self` is earlier.
    pub async fn test_instant_saturating_duration_since() {
        let dur_50ms = Duration::from_millis(50);

        let t1 = Rt::Instant::now();
        Rt::sleep(dur_50ms).await;
        let t2 = Rt::Instant::now();

        // t2.saturating_duration_since(t1) should be >= 50ms
        let duration = t2.saturating_duration_since(t1);
        assert!(duration >= dur_50ms);

        // t1.saturating_duration_since(t2) should be zero (t1 is earlier)
        let zero_duration = t1.saturating_duration_since(t2);
        assert_eq!(zero_duration, Duration::from_secs(0));
    }

    /// Test `Instant` ordering: `Ord`, `PartialOrd`, `Eq`, `PartialEq`.
    pub async fn test_instant_ord() {
        let dur_10ms = Duration::from_millis(10);

        let t1 = Rt::Instant::now();
        Rt::sleep(dur_10ms).await;
        let t2 = Rt::Instant::now();
        let t1_copy = t1;

        // PartialEq / Eq
        assert_eq!(t1, t1_copy);
        assert_ne!(t1, t2);

        // PartialOrd
        assert!(t1 < t2);
        assert!(t2 > t1);
        assert!(t1 <= t1_copy);
        assert!(t1 >= t1_copy);
        assert!(t1 <= t2);
        assert!(t2 >= t1);

        // Ord (via cmp)
        assert_eq!(t1.cmp(&t1_copy), std::cmp::Ordering::Equal);
        assert_eq!(t1.cmp(&t2), std::cmp::Ordering::Less);
        assert_eq!(t2.cmp(&t1), std::cmp::Ordering::Greater);
    }

    pub async fn test_sleep_until() {
        let start_time = Rt::Instant::now();
        let dur_10ms = Duration::from_millis(10);
        let end_time = start_time + dur_10ms;
        Rt::sleep_until(end_time).await;
        let elapsed = start_time.elapsed();
        assert!(elapsed >= dur_10ms);
    }

    pub async fn test_timeout() {
        let ret_number = 1;

        // Won't time out
        let dur_10ms = Duration::from_millis(10);
        let ret_value = Rt::timeout(dur_10ms, async move { ret_number }).await.unwrap();
        assert_eq!(ret_value, ret_number);

        // Will time out
        let dur_1s = Duration::from_secs(1);
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
        let dur_10ms = Duration::from_millis(10);
        let ddl = Rt::Instant::now() + dur_10ms;
        let ret_value = Rt::timeout_at(ddl, async move { ret_number }).await.unwrap();
        assert_eq!(ret_value, ret_number);

        // Will time out
        let dur_1s = Duration::from_secs(1);
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

    /// Test that `send()` returns `SendError` when receiver is dropped.
    pub async fn test_mpsc_send_to_closed_channel() {
        let (tx, rx) = Rt::Mpsc::channel::<i32>(5);
        drop(rx);

        let result = tx.send(42).await;
        assert!(result.is_err());

        // Verify the value is returned in the error
        let err = result.unwrap_err();
        assert_eq!(err.0, 42);
    }

    /// Test bounded channel backpressure: `send()` blocks when buffer is full.
    pub async fn test_mpsc_backpressure() {
        let buffer_size = 2;
        let (tx, mut rx) = Rt::Mpsc::channel::<i32>(buffer_size);

        // Fill the buffer
        tx.send(1).await.unwrap();
        tx.send(2).await.unwrap();

        // Next send should be pending (buffer is full)
        let send_fut = tx.send(3);
        let mut pinned_send_fut = pin!(send_fut);

        // Verify send is blocked
        assert!(
            matches!(poll_in_place(pinned_send_fut.as_mut()), Poll::Pending),
            "send() should be Pending when buffer is full"
        );

        // Receive one item to make room
        let received = rx.recv().await.unwrap();
        assert_eq!(received, 1);

        // Now the send should complete
        assert!(
            matches!(poll_in_place(pinned_send_fut.as_mut()), Poll::Ready(_)),
            "send() should be Ready after space is available"
        );

        // Verify remaining items
        assert_eq!(rx.recv().await.unwrap(), 2);
        assert_eq!(rx.recv().await.unwrap(), 3);
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

    /// Test that unbounded `send()` returns `SendError` when receiver is dropped.
    pub async fn test_unbounded_mpsc_send_to_closed_channel() {
        let (tx, rx) = Rt::MpscUnbounded::channel::<i32>();
        drop(rx);

        let result = tx.send(42);
        assert!(result.is_err());

        // Verify the value is returned in the error
        let err = result.unwrap_err();
        assert_eq!(err.0, 42);
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

        {
            assert!(is_pending(rx.changed()));
            tx.send(overwrite).unwrap();
            assert!(is_ready(rx.changed()));
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

    /// Test that `changed()` marks the value as seen after returning.
    ///
    /// This test verifies that after calling `borrow_watched()` followed by `changed()`,
    /// a subsequent call to `changed()` will properly wait for a new value instead of
    /// returning immediately (which would cause a hot loop / 100% CPU usage).
    pub async fn test_watch_changed_marks_as_seen() {
        let dur_50ms = Duration::from_millis(50);
        let dur_40ms = Duration::from_millis(40);

        let (tx, mut rx) = Rt::Watch::channel(0i32);

        // First, borrow the value (does not mark as seen)
        {
            let val = rx.borrow_watched();
            assert_eq!(*val, 0);
        }

        // Send a new value
        tx.send(1).unwrap();

        // First changed() should return immediately (value not yet seen)
        // and mark the value as seen
        assert!(is_ready(rx.changed()));

        // Verify we can see the new value
        {
            let val = rx.borrow_watched();
            assert_eq!(*val, 1);
        }

        // Clone tx for the spawned task
        let tx_clone = tx.clone();
        let _handle = Rt::spawn(async move {
            Rt::sleep(dur_50ms).await;
            tx_clone.send(2).unwrap();
        });

        // This changed() should wait for the new value (not return immediately)
        // If changed() doesn't properly mark as seen, this would return immediately
        // causing a hot loop
        let start = std::time::Instant::now();
        rx.changed().await.unwrap();
        let elapsed = start.elapsed();

        // Should have waited at least ~40ms for the new value
        assert!(
            elapsed >= dur_40ms,
            "changed() returned too quickly ({:?}), indicating it didn't wait for new value",
            elapsed
        );

        // Verify we got the new value
        {
            let val = rx.borrow_watched();
            assert_eq!(*val, 2);
        }

        drop(tx);
    }

    /// Test that `changed()` returns immediately when value has not been seen.
    ///
    /// Per the trait contract: "If the newest value in the channel has not yet been
    /// marked seen when this method is called, the method marks that value seen and
    /// returns immediately."
    pub async fn test_watch_changed_returns_immediately_when_unseen() {
        let (tx, mut rx) = Rt::Watch::channel(0i32);

        // Send a new value without reading the initial value
        tx.send(1).unwrap();

        // changed() should return immediately since the new value hasn't been seen
        assert!(is_ready(rx.changed()));

        // Verify the value
        {
            let val = rx.borrow_watched();
            assert_eq!(*val, 1);
        }
    }

    /// Test that multiple borrow_watched() calls don't affect changed() behavior.
    ///
    /// Since borrow_watched() uses borrow() which doesn't mark as seen,
    /// multiple calls shouldn't cause changed() to misbehave.
    pub async fn test_watch_multiple_borrow_then_changed() {
        let dur_50ms = Duration::from_millis(50);
        let dur_40ms = Duration::from_millis(40);

        let (tx, mut rx) = Rt::Watch::channel(0i32);

        // Multiple borrow_watched calls
        for _ in 0..5 {
            let val = rx.borrow_watched();
            assert_eq!(*val, 0);
        }

        // Send new value
        tx.send(1).unwrap();

        // changed() should return immediately since value hasn't been marked as seen
        assert!(is_ready(rx.changed()));

        {
            let val = rx.borrow_watched();
            assert_eq!(*val, 1);
        }

        // Now changed() should wait for the next value
        let tx_clone = tx.clone();
        let _handle = Rt::spawn(async move {
            Rt::sleep(dur_50ms).await;
            tx_clone.send(2).unwrap();
        });

        let start = std::time::Instant::now();
        rx.changed().await.unwrap();
        let elapsed = start.elapsed();

        assert!(elapsed >= dur_40ms, "changed() returned too quickly ({:?})", elapsed);

        {
            let val = rx.borrow_watched();
            assert_eq!(*val, 2);
        }

        drop(tx);
    }

    /// Test the wait loop pattern that openraft uses internally.
    ///
    /// This simulates the pattern used in openraft's Wait::metrics() method,
    /// ensuring changed() properly waits between iterations.
    pub async fn test_watch_wait_loop_pattern() {
        let dur_20ms = Duration::from_millis(20);

        let (tx, mut rx) = Rt::Watch::channel(0i32);

        // Spawn a task that increments the value periodically
        let tx_clone = tx.clone();
        let _handle = Rt::spawn(async move {
            for i in 1..=5 {
                Rt::sleep(dur_20ms).await;
                tx_clone.send(i).unwrap();
            }
        });

        // Wait until value reaches 3
        let target = 3;
        let mut iterations = 0;
        loop {
            {
                let current = rx.borrow_watched();
                if *current >= target {
                    assert_eq!(*current, 3);
                    break;
                }
            }
            rx.changed().await.unwrap();
            iterations += 1;

            // Safety check to prevent infinite loop in case of bug
            if iterations > 100 {
                panic!("Too many iterations, possible hot loop bug");
            }
        }

        // Should have taken only a few iterations (not 100s which would indicate hot loop)
        assert!(
            iterations <= 10,
            "Too many iterations ({}), possible hot loop",
            iterations
        );

        drop(tx);
    }

    /// Test `WatchReceiver::Clone` - multiple receivers can watch the same sender.
    pub async fn test_watch_multiple_receivers() {
        let (tx, rx1) = Rt::Watch::channel(0i32);
        let rx2 = rx1.clone();
        let rx3 = rx1.clone();

        // All receivers should see the initial value
        assert_eq!(*rx1.borrow_watched(), 0);
        assert_eq!(*rx2.borrow_watched(), 0);
        assert_eq!(*rx3.borrow_watched(), 0);

        // Send a new value
        tx.send(42).unwrap();

        // All receivers should see the new value
        assert_eq!(*rx1.borrow_watched(), 42);
        assert_eq!(*rx2.borrow_watched(), 42);
        assert_eq!(*rx3.borrow_watched(), 42);

        // Test that each receiver can independently wait for changes
        let (tx2, mut rx2_1) = Rt::Watch::channel(0i32);
        let mut rx2_2 = rx2_1.clone();

        // Spawn tasks that wait for changes on each receiver
        let handle1 = Rt::spawn(async move {
            rx2_1.changed().await.unwrap();
            *rx2_1.borrow_watched()
        });
        let handle2 = Rt::spawn(async move {
            rx2_2.changed().await.unwrap();
            *rx2_2.borrow_watched()
        });

        // Give spawned tasks time to start waiting
        Rt::sleep(Duration::from_millis(10)).await;

        // Send a value - both receivers should wake up
        tx2.send(100).unwrap();

        let val1 = handle1.await.unwrap();
        let val2 = handle2.await.unwrap();

        assert_eq!(val1, 100);
        assert_eq!(val2, 100);
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

    /// Test that oneshot `send()` returns `Err(T)` when receiver is dropped.
    pub async fn test_oneshot_send_to_dropped_rx() {
        let (tx, rx) = Rt::Oneshot::channel::<i32>();
        drop(rx);

        let result = tx.send(42);
        assert!(result.is_err());

        // Verify the value is returned in the error
        let returned_value = result.unwrap_err();
        assert_eq!(returned_value, 42);
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

/// Polls the future and returns its current state.
fn poll_in_place<F: Future>(fut: Pin<&mut F>) -> Poll<F::Output> {
    let waker = futures::task::noop_waker();
    let mut cx = futures::task::Context::from_waker(&waker);
    fut.poll(&mut cx)
}

/// Returns `true` if the future is ready when polled.
fn is_ready<F: Future>(fut: F) -> bool {
    let pinned = pin!(fut);
    matches!(poll_in_place(pinned), Poll::Ready(_))
}

/// Returns `true` if the future is pending when polled.
fn is_pending<F: Future>(fut: F) -> bool {
    let pinned = pin!(fut);
    matches!(poll_in_place(pinned), Poll::Pending)
}
