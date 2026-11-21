//! Suite for testing implementations of [`AsyncRuntime`].

#![allow(missing_docs)]

use std::future::Future;
use std::future::pending;
use std::future::poll_fn;
use std::future::ready;
use std::pin::Pin;
use std::pin::pin;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::task::Context;
use std::task::Poll;
use std::task::Wake;
use std::task::Waker;
use std::time::Duration;

use openraft_macros::since;

use crate::AsyncRuntime;
use crate::Instant;
use crate::Mutex;
use crate::Oneshot;
use crate::OneshotSender;
use crate::Select2;
use crate::Select3;
use crate::Select4;
use crate::SendError;
use crate::Watch;
use crate::mpsc::Mpsc;
use crate::mpsc::MpscReceiver;
use crate::mpsc::MpscSender;
use crate::mpsc::MpscWeakSender;
use crate::mpsc::TryRecvError;
use crate::watch::RecvError;
use crate::watch::WatchReceiver;
use crate::watch::WatchSender;

const MPSC_PEEK_CAPACITY: usize = 2;

mod buffered_receiver;

async fn ready_if<T>(enabled: bool, value: T) -> T {
    if !enabled {
        pending::<()>().await;
    }
    value
}

/// Test suite to ensure a runtime impl works as expected.
///
/// ```rust,ignore
/// struct MyCustomRuntime;
/// impl openraft::AsyncRuntime for MyCustomRuntime { /* omitted */ }
///
/// Suite::<MyCustomRuntime>::test_all();
/// ```
pub struct Suite<Rt: AsyncRuntime> {
    /// `Rt` needs to be used to make linter happy.
    _marker: std::marker::PhantomData<Rt>,
}

impl<Rt: AsyncRuntime> Suite<Rt> {
    pub fn test_all() {
        Self::test_block_on_non_send();

        let mut rt = Rt::new(1);
        Self::test_all_on(&mut rt);
        DetsimSuite::<Rt>::test_all();
    }

    /// Runs the conformance checks using the caller's runtime.
    ///
    /// [`Self::test_all`] also runs [`DetsimSuite`], which creates its own runtimes.
    #[since(version = "0.10.0")]
    pub fn test_all_on(rt: &mut Rt) {
        rt.block_on(async {
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
            Self::test_select2().await;
            Self::test_select3().await;
            Self::test_select4().await;
            Self::test_select_stops_polling().await;
            Self::test_select_wakes_from_later_branch().await;
            Self::test_select_bias_after_pending().await;
            Self::test_select_retains_borrowed_future().await;
            Self::test_select_drops_owned_future().await;
            Self::test_select_cancellation().await;
            #[cfg(feature = "single-threaded")]
            Self::test_select_non_send().await;

            Self::test_mpsc_recv_empty().await;
            Self::test_mpsc_recv_channel_closed().await;
            Self::test_mpsc_weak_sender_wont_prevent_channel_close().await;
            Self::test_mpsc_weak_sender_upgrade().await;
            Self::test_mpsc_send().await;
            Self::test_mpsc_send_to_closed_channel().await;
            Self::test_mpsc_backpressure().await;
            Self::test_mpsc_buffer_size_range().await;
            Self::test_buffered_receiver_backlog().await;
            Self::test_buffered_receiver_empty_close().await;
            Self::test_buffered_receiver_drop_idle().await;
            Self::test_buffered_receiver_drop_backlog().await;
            Self::test_buffered_receiver_cancel_receive().await;
            Self::test_buffered_receiver_keeps_a_pending_receive().await;
            #[cfg(feature = "single-threaded")]
            Self::test_buffered_receiver_non_send().await;
            Self::test_mpsc_try_peek_preserves_order().await;
            Self::test_mpsc_try_peek_empty().await;
            Self::test_mpsc_try_peek_closed().await;
            Self::test_mpsc_try_peek_cancelled_receive().await;
            Self::test_mpsc_try_peek_cancelled_pending_receive().await;
            Self::test_mpsc_try_peek_non_sync().await;
            #[cfg(feature = "single-threaded")]
            Self::test_mpsc_try_peek_non_send().await;
            Self::test_mpsc_peek_waits_for_item().await;
            Self::test_mpsc_peek_wakes_on_send().await;
            Self::test_mpsc_peek_closed().await;
            Self::test_mpsc_peek_wakes_on_close().await;

            Self::test_watch_init_value().await;
            Self::test_watch_overwrite_init_value().await;
            Self::test_watch_send_error_no_receiver().await;
            Self::test_watch_send_if_modified().await;
            Self::test_watch_wait_until_ge().await;
            Self::test_watch_wait_until().await;
            Self::test_watch_changed_marks_as_seen().await;
            Self::test_watch_borrow_and_update_marks_seen().await;
            Self::test_watch_changed_returns_immediately_when_unseen().await;
            Self::test_watch_changed_reports_final_value_before_close().await;
            Self::test_watch_multiple_borrow_then_changed().await;
            Self::test_watch_wait_loop_pattern().await;
            Self::test_watch_multiple_receivers().await;
            Self::test_watch_subscribe().await;
            Self::test_watch_send_if_different().await;
            Self::test_watch_send_if_greater().await;
            Self::test_oneshot_drop_tx().await;
            Self::test_oneshot().await;
            Self::test_oneshot_send_from_another_task().await;
            Self::test_oneshot_send_to_dropped_rx().await;
            Self::test_mutex().await;
            Self::test_mutex_contention().await;
            Self::test_mutex_lock_owned().await;

            Self::test_task_local().await;
            Self::test_task_local_on_completion_drop().await;
            Self::test_task_local_take_value().await;
            Self::test_task_local_poll_after_take_value().await;
            Self::test_task_local_get_value().await;
            Self::test_cancellable_completion().await;
            Self::test_cancellable_cancel().await;
            Self::test_cancellable_by_completion().await;
            Self::test_cancellable_by_cancel().await;
        });
    }

    fn test_block_on_non_send() {
        let expected = Rc::new(42);
        let mut rt = Rt::new(1);
        let future = async { expected.clone() };
        let result = rt.block_on(future);
        let shares_value = Rc::ptr_eq(&result, &expected);
        assert!(shares_value);

        let future = async { expected.clone() };
        let result = Rt::run(future);
        let shares_value = Rc::ptr_eq(&result, &expected);
        assert!(shares_value);
    }

    pub async fn test_spawn_join_handle() {
        for ret_number in 0..10 {
            let handle = Rt::spawn(async move { ret_number });
            let ret_value = handle.await.unwrap();
            assert_eq!(ret_value, ret_number);
        }
    }

    /// The runtime's RNG can sample values, and `random_range()` stays within its bounds.
    ///
    /// This is only a smoke check: the RNG of a runtime is not seeded, so its sequence cannot be
    /// checked. [`DetsimSuite`] checks the sequence of a seeded RNG.
    pub async fn test_thread_rng() {
        use rand::RngExt;

        let mut rng = Rt::thread_rng();

        let _: u32 = rng.random();
        let _: bool = rng.random();

        for _ in 0..100 {
            let value: u32 = rng.random_range(0..100);
            assert!(value < 100, "random_range should respect upper bound");
        }
    }

    pub async fn test_sleep() {
        let start_time = Rt::Instant::now();
        let dur_10ms = Duration::from_millis(10);
        let sleep = Rt::sleep(dur_10ms);
        let mut sleep = pin!(sleep);

        // On a loaded host the deadline can pass before the first poll, so either result is valid.
        let poll = futures_util::poll!(sleep.as_mut());
        if poll.is_pending() {
            sleep.await;
        }
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

        // Both instants come from one sample, so the difference is exact.
        let t1 = Rt::Instant::now();
        let t2 = t1 + dur_50ms;

        assert_eq!(t2 - t1, dur_50ms);
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

    async fn test_select2() {
        let expected = [Select2::First(1_u8), Select2::Second("two")];
        for (first_ready, expected) in expected.into_iter().enumerate() {
            let first = ready_if(first_ready == 0, 1_u8);
            let second = ready("two");
            let selected = Rt::select2(first, second).await;
            assert_eq!(selected, expected);
        }
    }

    async fn test_select3() {
        let expected = [Select3::First(1_u8), Select3::Second("two"), Select3::Third(true)];
        for (first_ready, expected) in expected.into_iter().enumerate() {
            let first = ready_if(first_ready == 0, 1_u8);
            let second = ready_if(first_ready <= 1, "two");
            let third = ready(true);
            let selected = Rt::select3(first, second, third).await;
            assert_eq!(selected, expected);
        }
    }

    async fn test_select4() {
        let expected = [
            Select4::First(1_u8),
            Select4::Second("two"),
            Select4::Third(true),
            Select4::Fourth([4, 5]),
        ];
        for (first_ready, expected) in expected.into_iter().enumerate() {
            let first = ready_if(first_ready == 0, 1_u8);
            let second = ready_if(first_ready <= 1, "two");
            let third = ready_if(first_ready <= 2, true);
            let fourth = ready([4, 5]);
            let selected = Rt::select4(first, second, third, fourth).await;
            assert_eq!(selected, expected);
        }
    }

    async fn test_select_stops_polling() {
        let later = || poll_fn(|_| -> Poll<()> { panic!("polled a future after a ready branch") });
        let second = later();
        let third = later();
        let fourth = later();
        let selected = Rt::select4(async { 1 }, second, third, fourth).await;
        let expected = Select4::First(1);
        assert_eq!(selected, expected);
    }

    async fn test_select_wakes_from_later_branch() {
        let (tx, rx) = Rt::Oneshot::channel::<i32>();
        let fourth = async { rx.await.unwrap() };
        let first = pending::<()>();
        let second = pending::<()>();
        let third = pending::<()>();
        let select = Rt::select4(first, second, third, fourth);
        let mut select = pin!(select);
        let polled = futures_util::poll!(select.as_mut());
        assert_eq!(polled, Poll::Pending);
        let sender = Rt::spawn(async move {
            Rt::sleep(Duration::from_millis(10)).await;
            tx.send(4).unwrap();
        });
        let selected = select.await;
        sender.await.unwrap();
        let expected = Select4::Fourth(4);
        assert_eq!(selected, expected);
    }

    async fn test_select_bias_after_pending() {
        let (first_tx, first_rx) = Rt::Oneshot::channel::<i32>();
        let (second_tx, second_rx) = Rt::Oneshot::channel::<i32>();
        let first = async { first_rx.await.unwrap() };
        let second = async { second_rx.await.unwrap() };
        let select = Rt::select2(first, second);
        let mut select = pin!(select);
        let polled = futures_util::poll!(select.as_mut());
        assert_eq!(polled, Poll::Pending);
        second_tx.send(2).unwrap();
        first_tx.send(1).unwrap();
        let selected = select.await;
        let expected = Select2::First(1);
        assert_eq!(selected, expected);
    }

    async fn test_select_retains_borrowed_future() {
        let mut second = pin!(async { "two" });
        let selected = Rt::select2(async { 1 }, second.as_mut()).await;
        let expected = Select2::First(1);
        assert_eq!(selected, expected);
        let retained = second.await;
        assert_eq!(retained, "two");
    }

    async fn test_select_drops_owned_future() {
        let marker = Arc::new(());
        let losing_marker = marker.clone();
        let second = ready_if(false, losing_marker);
        let selected = Rt::select2(async { 1 }, second).await;
        let expected = Select2::First(1);
        assert_eq!(selected, expected);
        let retained = Arc::strong_count(&marker);
        assert_eq!(retained, 1);
    }

    async fn test_select_cancellation() {
        let marker = Arc::new(());
        let first_marker = marker.clone();
        let second_marker = marker.clone();
        let first = ready_if(false, first_marker);
        let second = ready_if(false, second_marker);
        let select = Rt::select2(first, second);
        let mut select = Box::pin(select);
        let polled = futures_util::poll!(select.as_mut());
        assert_eq!(polled, Poll::Pending);
        drop(select);
        let retained = Arc::strong_count(&marker);
        assert_eq!(retained, 1);
    }

    #[cfg(feature = "single-threaded")]
    async fn test_select_non_send() {
        let value = std::rc::Rc::new(1);
        let first = async { value.clone() };
        let second = pending::<()>();
        let third = pending::<()>();
        let fourth = pending::<()>();
        let selected = Rt::select4(first, second, third, fourth).await;
        let expected = Select4::First(value);
        assert_eq!(selected, expected);
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

        // A live strong sender is enough to upgrade, even after the receiver is dropped; the
        // upgraded sender then returns the value it could not send.
        let (tx, rx) = Rt::Mpsc::channel::<u32>(5);
        let weak_tx = tx.downgrade();
        drop(rx);
        let upgraded = weak_tx.upgrade().expect("a strong sender is alive");
        let sent = upgraded.send(19).await;
        assert_eq!(Err(SendError(19)), sent);

        drop(upgraded);
        drop(tx);
        assert!(weak_tx.upgrade().is_none(), "no strong sender is left");
    }

    pub async fn test_mpsc_send() {
        let (tx, mut rx) = Rt::Mpsc::channel::<usize>(5);
        let tx = Arc::new(tx);

        let n_senders = 10_usize;

        let handles = (0..n_senders)
            .map(|idx| {
                let tx = tx.clone();
                Rt::spawn(async move { tx.send(idx).await })
            })
            .collect::<Vec<_>>();
        // Only the spawned tasks hold senders now, so the channel closes once they all end, even
        // when one fails before it sends.
        drop(tx);

        let mut received = Vec::with_capacity(n_senders);
        while let Some(value) = rx.recv().await {
            received.push(value);
        }

        for handle in handles {
            let sent = handle.await.unwrap();
            sent.unwrap();
        }

        received.sort();
        assert_eq!((0..n_senders).collect::<Vec<_>>(), received);
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

    /// `channel()` builds a channel at both ends of `buffer_size_range()`, which includes 1 on
    /// every runtime.
    #[since(version = "0.10.2")]
    pub async fn test_mpsc_buffer_size_range() {
        let range = Rt::Mpsc::buffer_size_range();
        assert!(
            range.contains(&1),
            "every runtime accepts a buffer of 1, got: {range:?}"
        );

        let _channel = Rt::Mpsc::channel::<u32>(*range.start());
        let _channel = Rt::Mpsc::channel::<u32>(*range.end());

        let (tx, mut rx) = Rt::Mpsc::channel::<u32>(1);
        tx.send(1).await.unwrap();
        assert_eq!(Some(1), rx.recv().await);
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

    async fn test_mpsc_try_peek_preserves_order() {
        let (tx, rx) = Rt::Mpsc::channel::<u64>(MPSC_PEEK_CAPACITY);
        let mut rx = rx.peekable();
        let sent = tx.send(10).await;
        sent.unwrap();
        let sent = tx.send(20).await;
        sent.unwrap();
        for _ in 0..2 {
            let peeked = rx.try_peek();
            assert_eq!(peeked, Ok(&10));
        }
        let first = rx.try_recv();
        assert_eq!(first, Ok(10));
        let second = rx.recv().await;
        assert_eq!(second, Some(20));
        let empty = rx.try_recv();
        assert_eq!(empty, Err(TryRecvError::Empty));
    }

    async fn test_mpsc_try_peek_empty() {
        let (tx, rx) = Rt::Mpsc::channel::<u64>(MPSC_PEEK_CAPACITY);
        let mut rx = rx.peekable();
        let empty = rx.try_peek();
        assert_eq!(empty, Err(TryRecvError::Empty));
        let sent = tx.send(10).await;
        sent.unwrap();
        let peeked = rx.try_peek();
        assert_eq!(peeked, Ok(&10));
        let received = rx.recv().await;
        assert_eq!(received, Some(10));
    }

    async fn test_mpsc_try_peek_closed() {
        let (tx, rx) = Rt::Mpsc::channel::<u64>(MPSC_PEEK_CAPACITY);
        let mut rx = rx.peekable();
        let sent = tx.send(10).await;
        sent.unwrap();
        let peeked = rx.try_peek();
        assert_eq!(peeked, Ok(&10));
        drop(tx);
        let peeked = rx.try_peek();
        assert_eq!(peeked, Ok(&10));
        let received = rx.recv().await;
        assert_eq!(received, Some(10));
        let closed = rx.try_peek();
        assert_eq!(closed, Err(TryRecvError::Disconnected));
        let closed = rx.try_recv();
        assert_eq!(closed, Err(TryRecvError::Disconnected));
        let received = rx.recv().await;
        assert_eq!(received, None);
    }

    async fn test_mpsc_try_peek_cancelled_receive() {
        let (tx, rx) = Rt::Mpsc::channel::<u64>(MPSC_PEEK_CAPACITY);
        let mut rx = rx.peekable();
        let sent = tx.send(10).await;
        sent.unwrap();
        let peeked = rx.try_peek();
        assert_eq!(peeked, Ok(&10));
        let shutdown = ready(());
        let receive = rx.recv();
        let selected = Rt::select2(shutdown, receive).await;
        assert_eq!(selected, Select2::First(()));
        let received = rx.recv().await;
        assert_eq!(received, Some(10));
    }

    async fn test_mpsc_try_peek_cancelled_pending_receive() {
        let (tx, rx) = Rt::Mpsc::channel::<u64>(MPSC_PEEK_CAPACITY);
        let mut rx = rx.peekable();
        {
            let receive = rx.recv();
            let mut receive = Box::pin(receive);
            let polled = futures_util::poll!(receive.as_mut());
            assert_eq!(polled, Poll::Pending);
        }
        let sent = tx.send(10).await;
        sent.unwrap();
        let peeked = rx.try_peek();
        assert_eq!(peeked, Ok(&10));
        let received = rx.recv().await;
        assert_eq!(received, Some(10));
    }

    async fn test_mpsc_try_peek_non_sync() {
        let (tx, rx) = Rt::Mpsc::channel::<std::cell::Cell<u64>>(MPSC_PEEK_CAPACITY);
        let mut rx = rx.peekable();
        let value = std::cell::Cell::new(10);
        let sent = tx.send(value).await;
        sent.unwrap();
        let peeked = rx.try_peek().unwrap();
        let value = peeked.get();
        assert_eq!(value, 10);
        let received = rx.recv().await;
        let value = std::cell::Cell::new(10);
        let expected = Some(value);
        assert_eq!(received, expected);
    }

    #[cfg(feature = "single-threaded")]
    async fn test_mpsc_try_peek_non_send() {
        let (tx, rx) = Rt::Mpsc::channel::<std::rc::Rc<u64>>(MPSC_PEEK_CAPACITY);
        let mut rx = rx.peekable();
        let value = std::rc::Rc::new(10);
        let sent_value = value.clone();
        let sent = tx.send(sent_value).await;
        sent.unwrap();
        let peeked = rx.try_peek();
        assert_eq!(peeked, Ok(&value));
        let received = rx.recv().await;
        assert_eq!(received, Some(value));
    }

    async fn test_mpsc_peek_waits_for_item() {
        let (tx, rx) = Rt::Mpsc::channel::<u64>(MPSC_PEEK_CAPACITY);
        let mut rx = rx.peekable();
        {
            let peek = rx.peek();
            let mut peek = Box::pin(peek);
            let polled = futures_util::poll!(peek.as_mut());
            assert_eq!(polled, Poll::Pending);
        }
        let sent = tx.send(10).await;
        sent.unwrap();
        for _ in 0..2 {
            let peeked = rx.peek().await;
            assert_eq!(peeked, Some(&10));
        }
        let received = rx.recv().await;
        assert_eq!(received, Some(10));
    }

    /// A pending peek is woken by a send, and then returns the sent item.
    async fn test_mpsc_peek_wakes_on_send() {
        let (tx, rx) = Rt::Mpsc::channel::<u64>(MPSC_PEEK_CAPACITY);
        let mut rx = rx.peekable();
        {
            let woken = Arc::new(WakeFlag::default());
            let waker = Waker::from(Arc::clone(&woken));
            let mut cx = Context::from_waker(&waker);
            let mut peek = pin!(rx.peek());
            let polled = peek.as_mut().poll(&mut cx);
            assert!(polled.is_pending(), "a peek on an empty live channel should be pending");
            let sent = tx.send(10).await;
            sent.unwrap();
            assert!(woken.is_set(), "a send should wake the pending peek");
            let peeked = Rt::timeout(Duration::from_secs(1), peek).await.unwrap();
            assert_eq!(peeked, Some(&10));
        }
        let received = rx.recv().await;
        assert_eq!(received, Some(10));
        let empty = rx.try_recv();
        assert_eq!(empty, Err(TryRecvError::Empty));
    }

    async fn test_mpsc_peek_closed() {
        let (tx, rx) = Rt::Mpsc::channel::<u64>(MPSC_PEEK_CAPACITY);
        let mut rx = rx.peekable();
        let sent = tx.send(10).await;
        sent.unwrap();
        drop(tx);
        let peeked = rx.peek().await;
        assert_eq!(peeked, Some(&10));
        let received = rx.recv().await;
        assert_eq!(received, Some(10));
        let closed = rx.peek().await;
        assert_eq!(closed, None);
    }

    /// A pending peek is woken when the last sender is dropped, and then returns `None`.
    async fn test_mpsc_peek_wakes_on_close() {
        let (tx, rx) = Rt::Mpsc::channel::<u64>(MPSC_PEEK_CAPACITY);
        let mut rx = rx.peekable();
        let woken = Arc::new(WakeFlag::default());
        let waker = Waker::from(Arc::clone(&woken));
        let mut cx = Context::from_waker(&waker);
        let mut peek = pin!(rx.peek());
        let polled = peek.as_mut().poll(&mut cx);
        assert!(polled.is_pending(), "a peek on an empty live channel should be pending");
        drop(tx);
        assert!(woken.is_set(), "dropping the last sender should wake the pending peek");
        let closed = Rt::timeout(Duration::from_secs(1), peek).await.unwrap();
        assert_eq!(closed, None);
    }

    pub async fn test_watch_init_value() {
        let init_value = 1;
        let (tx, mut rx) = Rt::Watch::channel(init_value);
        assert!(is_pending(rx.changed()));
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
        let (tx, rx) = Rt::Watch::channel(1_u64);
        drop(rx);
        let sent = tx.send(2);
        assert_eq!(Err(crate::watch::SendError(2)), sent);
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
        let (tx, mut rx) = Rt::Watch::channel(0i32);

        // First, borrow the value (does not mark as seen)
        {
            let val = rx.borrow_watched();
            assert_eq!(*val, 0);
        }

        // Send a new value
        tx.send(1).unwrap();

        // Reading before `change()` does not invalidate the following `change()` to pending
        {
            let val = rx.borrow_watched();
            assert_eq!(*val, 1);
        }

        // First changed() should return immediately (value not yet seen)
        // and mark the value as seen
        assert!(is_ready(rx.changed()));
        // Second change should return pending, because previous changed() mark it as seen.
        assert!(is_pending(rx.changed()));

        // Verify we can see the new value
        {
            let val = rx.borrow_watched();
            assert_eq!(*val, 1);
        }

        // This changed() must wait for a newer value: if the earlier changed() had not marked 1
        // as seen, it would return at once.
        {
            let mut changed = pin!(rx.changed());
            let polled = futures_util::poll!(changed.as_mut());
            assert!(polled.is_pending(), "changed() should wait once the value is seen");
            tx.send(2).unwrap();
            let changed = Rt::timeout(Duration::from_secs(1), changed).await.unwrap();
            changed.unwrap();
        }

        // Verify we got the new value
        {
            let val = rx.borrow_watched();
            assert_eq!(*val, 2);
        }

        drop(tx);
    }

    /// Test that `borrow_and_update()` returns the latest value and marks it as seen.
    ///
    /// Per the trait contract: after `borrow_and_update()`, a following `changed()`
    /// sleeps until a newer value is sent, instead of returning immediately for the
    /// value already returned.
    pub async fn test_watch_borrow_and_update_marks_seen() {
        let (tx, mut rx) = Rt::Watch::channel(0i32);

        // A plain borrow does not mark the value as seen: changed() still fires.
        tx.send(1).unwrap();
        {
            let val = rx.borrow_watched();
            assert_eq!(*val, 1);
        }
        assert!(is_ready(rx.changed()));

        // borrow_and_update() returns the latest value and marks it as seen.
        tx.send(2).unwrap();
        {
            let val = rx.borrow_and_update();
            assert_eq!(*val, 2);
        }
        assert!(is_pending(rx.changed()));

        // A newer value makes changed() fire again.
        tx.send(3).unwrap();
        assert!(is_ready(rx.changed()));

        // On an already-seen value, borrow_and_update() still returns it, and
        // changed() stays pending.
        {
            let val = rx.borrow_and_update();
            assert_eq!(*val, 3);
        }
        assert!(is_pending(rx.changed()));

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

    /// Test that `changed()` reports a value sent just before the last sender is dropped, once,
    /// and then the closure.
    ///
    /// As with tokio's watch channel, every receiver sees the final value before `changed()`
    /// returns [`RecvError`], including a receiver that was already waiting when it was sent.
    #[since(version = "0.10.2")]
    pub async fn test_watch_changed_reports_final_value_before_close() {
        // A waiting `changed()` and an idle receiver each see the final value once.
        {
            let (tx, mut rx) = Rt::Watch::channel(0i32);
            let mut rx2 = rx.clone();
            {
                let mut changed = pin!(rx.changed());
                assert!(poll_in_place(changed.as_mut()).is_pending());

                tx.send(1).unwrap();
                drop(tx);

                assert_eq!(Ok(()), changed.await);
            }
            assert_eq!(1, *rx.borrow_watched());
            assert_eq!(Err(RecvError(())), rx.changed().await);

            assert_eq!(Ok(()), rx2.changed().await);
            assert_eq!(1, *rx2.borrow_watched());
            assert_eq!(Err(RecvError(())), rx2.changed().await);
        }

        // A waiting `wait_until_ge()` returns the final value.
        {
            let (tx, mut rx) = Rt::Watch::channel(0i32);
            let mut wait = pin!(rx.wait_until_ge(&1));
            assert!(poll_in_place(wait.as_mut()).is_pending());

            tx.send(1).unwrap();
            drop(tx);

            assert_eq!(Ok(1), wait.await);
        }

        // A receiver that has seen every value gets the closure at once.
        {
            let (tx, mut rx) = Rt::Watch::channel(0i32);
            drop(tx);
            assert_eq!(Err(RecvError(())), rx.changed().await);
        }
    }

    /// Test that multiple borrow_watched() calls don't affect changed() behavior.
    ///
    /// Since borrow_watched() uses borrow() which doesn't mark as seen,
    /// multiple calls shouldn't cause changed() to misbehave.
    pub async fn test_watch_multiple_borrow_then_changed() {
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
        {
            let mut changed = pin!(rx.changed());
            let polled = futures_util::poll!(changed.as_mut());
            assert!(polled.is_pending(), "changed() should wait once the value is seen");
            tx.send(2).unwrap();
            let changed = Rt::timeout(Duration::from_secs(1), changed).await.unwrap();
            changed.unwrap();
        }

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
                tx_clone.send(i).ok();
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
            "Too many iterations ({iterations}), possible hot loop"
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

    /// Test `WatchSender::subscribe()` - create a new receiver from the sender.
    pub async fn test_watch_subscribe() {
        let (tx, rx1) = Rt::Watch::channel(0i32);

        // Create a new receiver via subscribe()
        let rx2 = tx.subscribe();

        // Both receivers should see the initial value
        assert_eq!(*rx1.borrow_watched(), 0);
        assert_eq!(*rx2.borrow_watched(), 0);

        // Send a new value
        tx.send(42).unwrap();

        // All receivers should see the new value
        assert_eq!(*rx1.borrow_watched(), 42);
        assert_eq!(*rx2.borrow_watched(), 42);

        // Create another receiver after sending a value
        let rx3 = tx.subscribe();
        assert_eq!(*rx3.borrow_watched(), 42);

        // Test that subscribed receivers can independently wait for changes
        let mut rx4 = tx.subscribe();
        let mut rx5 = tx.subscribe();

        let handle1 = Rt::spawn(async move {
            rx4.changed().await.unwrap();
            *rx4.borrow_watched()
        });
        let handle2 = Rt::spawn(async move {
            rx5.changed().await.unwrap();
            *rx5.borrow_watched()
        });

        // Give spawned tasks time to start waiting
        Rt::sleep(Duration::from_millis(10)).await;

        // Send a value - both receivers should wake up
        tx.send(100).unwrap();

        let val1 = handle1.await.unwrap();
        let val2 = handle2.await.unwrap();

        assert_eq!(val1, 100);
        assert_eq!(val2, 100);
    }

    /// Test `WatchSender::send_if_different()` - only sends when value differs.
    pub async fn test_watch_send_if_different() {
        let (tx, mut rx) = Rt::Watch::channel(0i32);

        // Sending the same value should return false and not notify receivers
        let updated = tx.send_if_different(0);
        assert!(!updated);

        // changed() should be pending since value wasn't updated
        assert!(is_pending(rx.changed()));

        // Sending a different value should return true
        let updated = tx.send_if_different(42);
        assert!(updated);
        assert_eq!(*tx.borrow_watched(), 42);

        // changed() should be ready since value was updated
        assert!(is_ready(rx.changed()));
        assert_eq!(*rx.borrow_watched(), 42);

        // Sending the same value again should return false
        let updated = tx.send_if_different(42);
        assert!(!updated);

        // changed() should be pending since value wasn't updated
        assert!(is_pending(rx.changed()));

        // Sending another different value should return true
        let updated = tx.send_if_different(100);
        assert!(updated);
        assert_eq!(*tx.borrow_watched(), 100);

        // changed() should be ready
        assert!(is_ready(rx.changed()));
        assert_eq!(*rx.borrow_watched(), 100);
    }

    /// Test `WatchSender::send_if_greater()` - only sends when value is greater.
    pub async fn test_watch_send_if_greater() {
        let (tx, mut rx) = Rt::Watch::channel(10i32);

        // Sending a smaller value should return false and not notify receivers
        let updated = tx.send_if_greater(5);
        assert!(!updated);
        assert_eq!(*tx.borrow_watched(), 10);

        // changed() should be pending since value wasn't updated
        assert!(is_pending(rx.changed()));

        // Sending an equal value should return false
        let updated = tx.send_if_greater(10);
        assert!(!updated);
        assert_eq!(*tx.borrow_watched(), 10);

        // changed() should still be pending
        assert!(is_pending(rx.changed()));

        // Sending a greater value should return true
        let updated = tx.send_if_greater(42);
        assert!(updated);
        assert_eq!(*tx.borrow_watched(), 42);

        // changed() should be ready since value was updated
        assert!(is_ready(rx.changed()));
        assert_eq!(*rx.borrow_watched(), 42);

        // Sending a smaller value again should return false
        let updated = tx.send_if_greater(20);
        assert!(!updated);
        assert_eq!(*tx.borrow_watched(), 42);

        // changed() should be pending
        assert!(is_pending(rx.changed()));

        // Sending another greater value should return true
        let updated = tx.send_if_greater(100);
        assert!(updated);
        assert_eq!(*tx.borrow_watched(), 100);

        // changed() should be ready
        assert!(is_ready(rx.changed()));
        assert_eq!(*rx.borrow_watched(), 100);
    }

    pub async fn test_oneshot_drop_tx() {
        // A receiver first polled after the sender is dropped resolves to an error.
        let (tx, rx) = Rt::Oneshot::channel::<()>();
        drop(tx);
        assert!(rx.await.is_err());

        // A waiting receiver is woken by the drop, and then resolves to an error, possibly only
        // after its task yields.
        let (tx, mut rx) = Rt::Oneshot::channel::<()>();
        let woken = Arc::new(WakeFlag::default());
        let waker = Waker::from(Arc::clone(&woken));
        let mut cx = Context::from_waker(&waker);
        let polled = Pin::new(&mut rx).poll(&mut cx);
        assert!(polled.is_pending(), "a receiver with a live sender should be pending");
        drop(tx);
        assert!(woken.is_set(), "dropping the sender should wake the waiting receiver");
        let received = Rt::timeout(Duration::from_secs(1), rx).await.unwrap();
        assert!(received.is_err());
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

    /// Test `lock_owned()` returns a guard that owns the mutex via Arc.
    pub async fn test_mutex_lock_owned() {
        // Test basic lock_owned functionality
        {
            let mutex = Arc::new(Rt::Mutex::new(42_i32));
            let guard = Arc::clone(&mutex).lock_owned().await;
            assert_eq!(*guard, 42);
        }

        // Test that the guard can be moved and returned from async blocks
        {
            let mutex = Arc::new(Rt::Mutex::new(100_i32));
            let guard = async { mutex.lock_owned().await }.await;
            assert_eq!(*guard, 100);
        }

        // Test that lock_owned prevents concurrent access
        let mutex = Arc::new(Rt::Mutex::new(0_u32));
        let mutex1 = Arc::clone(&mutex);
        let guard = mutex1.lock_owned().await;

        // Try to acquire another lock - should be pending
        let lock_fut = mutex.lock();
        let mut pinned_lock_fut = pin!(lock_fut);
        assert!(matches!(poll_in_place(pinned_lock_fut.as_mut()), Poll::Pending));

        // Drop the owned guard and the lock should succeed
        drop(guard);
        assert!(matches!(poll_in_place(pinned_lock_fut), Poll::Ready(_)));
    }

    /// Test basic task_local scope, get, and with.
    pub async fn test_task_local() {
        crate::task_local! {
            static REQ_ID: u32;
            pub static FOO: bool;
        }

        let j1 = Rt::spawn(REQ_ID.scope(1, async move {
            assert_eq!(REQ_ID.get(), 1);
            assert_eq!(REQ_ID.get(), 1);
        }));

        let j2 = Rt::spawn(REQ_ID.scope(2, async move {
            REQ_ID.with(|v| {
                assert_eq!(REQ_ID.get(), 2);
                assert_eq!(*v, 2);
            });

            Rt::sleep(Duration::from_millis(10)).await;

            assert_eq!(REQ_ID.get(), 2);
        }));

        let j3 = Rt::spawn(FOO.scope(true, async move {
            assert!(FOO.get());
        }));

        j1.await.unwrap();
        j2.await.unwrap();
        j3.await.unwrap();
    }

    /// Test that task-local is available when a future is dropped on completion.
    pub async fn test_task_local_on_completion_drop() {
        crate::task_local! {
            static KEY: u32;
        }

        struct MyFuture<Rt: AsyncRuntime> {
            tx: Option<<Rt::Oneshot as Oneshot>::Sender<u32>>,
        }
        impl<Rt: AsyncRuntime> Future for MyFuture<Rt> {
            type Output = ();

            fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
                Poll::Ready(())
            }
        }
        impl<Rt: AsyncRuntime> Drop for MyFuture<Rt> {
            fn drop(&mut self) {
                let _ = self.tx.take().unwrap().send(KEY.get());
            }
        }

        let (tx, rx) = Rt::Oneshot::channel();

        let h = Rt::spawn(KEY.scope(42, MyFuture::<Rt> { tx: Some(tx) }));

        assert_eq!(rx.await.unwrap(), 42);
        h.await.unwrap();
    }

    /// Test `TaskLocalFuture::take_value`.
    pub async fn test_task_local_take_value() {
        crate::task_local! {
            static KEY: u32;
        }
        let fut = KEY.scope(1, async {});
        let mut pinned = Box::pin(fut);
        assert_eq!(pinned.as_mut().take_value(), Some(1));
        assert_eq!(pinned.as_mut().take_value(), None);
    }

    /// Test that polling after `take_value` sees no task-local value.
    pub async fn test_task_local_poll_after_take_value() {
        crate::task_local! {
            static KEY: u32;
        }
        let fut = KEY.scope(1, async {
            let result = KEY.try_with(|_| {});
            assert!(result.is_err());
        });
        let mut fut = Box::pin(fut);
        fut.as_mut().take_value();

        fut.await;
    }

    /// Test `LocalKey::get` and `LocalKey::try_get`.
    pub async fn test_task_local_get_value() {
        crate::task_local! {
            static KEY: u32;
        }

        KEY.scope(1, async {
            assert_eq!(KEY.get(), 1);
            assert_eq!(KEY.try_get().unwrap(), 1);
        })
        .await;

        let fut = KEY.scope(1, async {
            let result = KEY.try_get();
            assert!(result.is_err());
        });
        let mut fut = Box::pin(fut);
        fut.as_mut().take_value();

        fut.await;
    }

    async fn test_cancellable_completion() {
        let (mut cancel, future) = Rt::cancellable(async { 7 });
        let actual = future.await;
        assert_eq!(Some(7), actual);
        cancel.cancel();
    }

    async fn test_cancellable_cancel() {
        let (mut cancel, future) = Rt::cancellable(pending::<u64>());
        cancel.cancel();
        let actual = Rt::timeout(Duration::from_secs(1), future).await.unwrap();
        assert_eq!(None, actual);
    }

    async fn test_cancellable_by_completion() {
        let future = Rt::cancellable_by(async { 7 }, pending::<()>());
        let actual = future.await;
        assert_eq!(Some(7), actual);
    }

    async fn test_cancellable_by_cancel() {
        let future = Rt::cancellable_by(async { 7 }, async { "cancel" });
        let actual = future.await;
        assert_eq!(None, actual);
    }
}

/// A waker that records whether it was woken.
#[derive(Default)]
struct WakeFlag(AtomicBool);

impl WakeFlag {
    fn is_set(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

impl Wake for WakeFlag {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// Polls the future and returns its current state.
fn poll_in_place<F: Future>(fut: Pin<&mut F>) -> Poll<F::Output> {
    let waker = futures_util::task::noop_waker();
    let mut cx = futures_util::task::Context::from_waker(&waker);
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

/// Test suite for [`DeterministicRng`](crate::deterministic_rng::DeterministicRng) properties.
///
/// Each test is self-contained: creates its own `DeterministicRng` runtime,
/// sets a seed, and runs `block_on`.
pub struct DetsimSuite<Rt: AsyncRuntime> {
    _marker: std::marker::PhantomData<Rt>,
}

type Det<Rt> = crate::deterministic_rng::DeterministicRng<Rt>;

impl<Rt: AsyncRuntime> DetsimSuite<Rt> {
    fn new_runtime(seed: u64) -> Det<Rt> {
        let mut rt = Det::<Rt>::new(1);
        rt.set_seed(seed);
        rt
    }

    pub fn test_all() {
        Self::test_thread_rng_determinism();
        Self::test_thread_rng_seeded_sequence();
        Self::test_thread_rng_sequence_advances();
        Self::test_spawn_seed_differs_from_parent();
        Self::test_two_spawned_tasks_differ();
        Self::test_scope();
        Self::test_block_on_seed_is_only_for_the_root();
        Self::test_nested_wrapper_uses_its_own_seed();
        Self::test_seed_continues_across_block_on();
    }

    /// Same seed produces the same RNG sequence; different seed differs.
    fn test_thread_rng_determinism() {
        use rand::RngExt;

        let collect = |seed: u64| -> Vec<u64> {
            Self::new_runtime(seed)
                .block_on(async { (0..5).map(|_| Det::<Rt>::thread_rng().random::<u64>()).collect() })
        };

        let run1 = collect(123);
        let run2 = collect(123);
        assert_eq!(run1, run2, "same seed should produce the same RNG sequence");

        let run3 = collect(456);
        assert_ne!(run1, run3, "different seeds should produce different sequences");
    }

    /// Under a seed, the first `thread_rng()` of the root draws the sequence of a `SmallRng`
    /// seeded with that seed.
    ///
    /// The control's draws differ from each other, so a generator that repeats its first draw
    /// fails the comparison.
    fn test_thread_rng_seeded_sequence() {
        use rand::RngExt;
        use rand::SeedableRng;
        use rand::rngs::SmallRng;

        let mut control = SmallRng::seed_from_u64(42);
        let expected: [u64; 3] = [control.random(), control.random(), control.random()];
        assert_ne!(expected[0], expected[1]);
        assert_ne!(expected[0], expected[2]);

        let drawn: [u64; 3] = Self::new_runtime(42).block_on(async {
            let mut rng = Det::<Rt>::thread_rng();
            [rng.random(), rng.random(), rng.random()]
        });
        assert_eq!(expected, drawn, "the seeded RNG should draw the control's sequence");
    }

    /// Consecutive `thread_rng()` calls produce different values.
    fn test_thread_rng_sequence_advances() {
        use rand::RngExt;

        Self::new_runtime(42).block_on(async {
            let v1: u64 = Det::<Rt>::thread_rng().random();
            let v2: u64 = Det::<Rt>::thread_rng().random();
            assert_ne!(v1, v2, "consecutive thread_rng() calls should produce different values");
        });
    }

    /// A spawned task's RNG differs from the parent's.
    fn test_spawn_seed_differs_from_parent() {
        use rand::RngExt;

        Self::new_runtime(42).block_on(async {
            let (tx, rx) = <Det<Rt> as AsyncRuntime>::Oneshot::channel::<u64>();

            #[allow(clippy::let_underscore_future)]
            let _ = Det::<Rt>::spawn(async move {
                let _ = tx.send(Det::<Rt>::thread_rng().random::<u64>());
            });

            let child_val: u64 = rx.await.unwrap();
            let parent_val: u64 = Det::<Rt>::thread_rng().random();

            assert_ne!(
                parent_val, child_val,
                "parent and child should get different RNG values"
            );
        });
    }

    /// Two separately spawned tasks get different RNG values.
    fn test_two_spawned_tasks_differ() {
        use rand::RngExt;

        Self::new_runtime(42).block_on(async {
            let (tx1, rx1) = <Det<Rt> as AsyncRuntime>::Oneshot::channel::<u64>();
            let (tx2, rx2) = <Det<Rt> as AsyncRuntime>::Oneshot::channel::<u64>();

            #[allow(clippy::let_underscore_future)]
            let _ = Det::<Rt>::spawn(async move {
                let _ = tx1.send(Det::<Rt>::thread_rng().random::<u64>());
            });

            #[allow(clippy::let_underscore_future)]
            let _ = Det::<Rt>::spawn(async move {
                let _ = tx2.send(Det::<Rt>::thread_rng().random::<u64>());
            });

            let v1: u64 = rx1.await.unwrap();
            let v2: u64 = rx2.await.unwrap();

            assert_ne!(v1, v2, "two spawned tasks should get different RNG values");
        });
    }

    /// `scope()` sets the seed for an async future without `block_on`.
    fn test_scope() {
        use rand::RngExt;

        Self::new_runtime(0).block_on(async {
            // Same seed produces same value
            let v1: u64 = Det::<Rt>::scope(99, async { Det::<Rt>::thread_rng().random() }).await;
            let v2: u64 = Det::<Rt>::scope(99, async { Det::<Rt>::thread_rng().random() }).await;
            assert_eq!(v1, v2, "scope with same seed should produce same value");

            // Different seed produces different value
            let v3: u64 = Det::<Rt>::scope(100, async { Det::<Rt>::thread_rng().random() }).await;
            assert_ne!(v1, v3, "scope with different seed should produce different value");
        });
    }

    /// A task spawned through the inner runtime cannot draw from the seed of `block_on`, so the
    /// root keeps its sequence.
    fn test_block_on_seed_is_only_for_the_root() {
        use rand::RngExt;

        let control: u64 = Self::new_runtime(17).block_on(async { Det::<Rt>::thread_rng().random() });

        let (unscoped_drew, root_first) = Self::new_runtime(17).block_on(async {
            let unscoped = Rt::spawn(async { std::panic::catch_unwind(Det::<Rt>::thread_rng).is_ok() });
            let unscoped_drew = unscoped.await.unwrap();
            let root_first: u64 = Det::<Rt>::thread_rng().random();
            (unscoped_drew, root_first)
        });

        assert!(
            !unscoped_drew,
            "a task spawned through the inner runtime should have no seed"
        );
        assert_eq!(control, root_first, "the root should get its first draw");
    }

    /// A wrapper around another `DeterministicRng` draws from its own seed, not the inner one.
    fn test_nested_wrapper_uses_its_own_seed() {
        use rand::RngExt;

        for seed in [17, 87] {
            let control: u64 = Self::new_runtime(seed).block_on(async { Det::<Rt>::thread_rng().random() });

            let mut nested = Det::<Det<Rt>>::new(1);
            nested.set_seed(seed);
            let got: u64 = nested.block_on(async { Det::<Det<Rt>>::thread_rng().random() });

            assert_eq!(control, got, "the nested wrapper should use seed {seed}");
        }
    }

    /// The sequence continues across `block_on` calls, with a root future that is not `Send`.
    fn test_seed_continues_across_block_on() {
        use rand::RngExt;

        let draw = || async {
            let not_send = Rc::new(());
            ready(()).await;
            let value: u64 = Det::<Rt>::thread_rng().random();
            drop(not_send);
            value
        };

        let uninterrupted: [u64; 2] = Self::new_runtime(17).block_on(async { [draw().await, draw().await] });

        let mut rt = Self::new_runtime(17);
        let split = [rt.block_on(draw()), rt.block_on(draw())];

        assert_eq!(uninterrupted, split, "a second block_on should continue the sequence");
    }
}
