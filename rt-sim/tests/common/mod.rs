//! The conformance suite's sub-tests in `Suite::test_all` order, on one caller-owned runtime, so
//! the caller can read the trace. (`Suite::test_all` creates its own runtime.)

use openraft_rt::testing::Suite;
use openraft_rt_sim::SimRuntime;

/// Runs every `Suite` sub-test in `test_all` order.
pub async fn run_suite() {
    type S = Suite<SimRuntime>;

    S::test_spawn_join_handle().await;
    S::test_thread_rng().await;
    S::test_sleep().await;
    S::test_instant().await;
    S::test_instant_arithmetic().await;
    S::test_instant_sub_instant().await;
    S::test_instant_saturating_duration_since().await;
    S::test_instant_ord().await;
    S::test_sleep_until().await;
    S::test_timeout().await;
    S::test_timeout_at().await;

    S::test_mpsc_recv_empty().await;
    S::test_mpsc_recv_channel_closed().await;
    S::test_mpsc_weak_sender_wont_prevent_channel_close().await;
    S::test_mpsc_weak_sender_upgrade().await;
    S::test_mpsc_send().await;
    S::test_mpsc_send_to_closed_channel().await;
    S::test_mpsc_backpressure().await;

    S::test_watch_init_value().await;
    S::test_watch_overwrite_init_value().await;
    S::test_watch_send_error_no_receiver().await;
    S::test_watch_send_if_modified().await;
    S::test_watch_wait_until_ge().await;
    S::test_watch_wait_until().await;
    S::test_watch_changed_marks_as_seen().await;
    S::test_watch_borrow_and_update_marks_seen().await;
    S::test_watch_changed_returns_immediately_when_unseen().await;
    S::test_watch_multiple_borrow_then_changed().await;
    S::test_watch_wait_loop_pattern().await;
    S::test_watch_multiple_receivers().await;
    S::test_watch_subscribe().await;
    S::test_watch_send_if_different().await;
    S::test_watch_send_if_greater().await;
    S::test_oneshot_drop_tx().await;
    S::test_oneshot().await;
    S::test_oneshot_send_from_another_task().await;
    S::test_oneshot_send_to_dropped_rx().await;
    S::test_mutex().await;
    S::test_mutex_contention().await;
    S::test_mutex_lock_owned().await;

    S::test_task_local().await;
    S::test_task_local_on_completion_drop().await;
    S::test_task_local_take_value().await;
    S::test_task_local_poll_after_take_value().await;
    S::test_task_local_get_value().await;
}
