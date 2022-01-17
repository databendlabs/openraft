use std::time::Duration;

use maplit::btreeset;
use tokio::sync::watch;
use tokio::time::sleep;

use crate::core::EffectiveMembership;
use crate::metrics::Wait;
use crate::metrics::WaitError;
use crate::raft_types::LogIdOptionExt;
use crate::LogId;
use crate::Membership;
use crate::RaftMetrics;
use crate::State;

/// Test wait for different state changes
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_wait() -> anyhow::Result<()> {
    {
        // wait for leader
        let (init, w, tx) = init_wait_test();

        let h = tokio::spawn(async move {
            sleep(Duration::from_millis(10)).await;
            let mut update = init.clone();
            update.current_leader = Some(3);
            let rst = tx.send(update);
            assert!(rst.is_ok());
        });
        let got = w.current_leader(3, "leader").await?;
        h.await?;
        assert_eq!(Some(3), got.current_leader);
    }

    {
        // wait for log
        let (init, w, tx) = init_wait_test();

        let h = tokio::spawn(async move {
            sleep(Duration::from_millis(10)).await;
            let mut update = init.clone();
            update.last_log_index = Some(3);
            update.last_applied = Some(LogId::new(1, 3));
            let rst = tx.send(update);
            assert!(rst.is_ok());
        });
        let got = w.log(Some(3), "log").await?;
        let got_least2 = w.log_at_least(2, "log").await?;
        let got_least3 = w.log_at_least(3, "log").await?;
        let got_least4 = w.log_at_least(4, "log").await;
        h.await?;

        assert_eq!(Some(3), got.last_log_index);
        assert_eq!(Some(3), got.last_applied.index());
        assert_eq!(Some(3), got_least2.last_log_index);
        assert_eq!(Some(3), got_least2.last_applied.index());
        assert_eq!(Some(3), got_least3.last_log_index);
        assert_eq!(Some(3), got_least3.last_applied.index());

        assert!(got_least4.is_err());
    }

    {
        // wait for state
        let (init, w, tx) = init_wait_test();

        let h = tokio::spawn(async move {
            sleep(Duration::from_millis(10)).await;
            let mut update = init.clone();
            update.state = State::Leader;
            let rst = tx.send(update);
            assert!(rst.is_ok());
        });
        let got = w.state(State::Leader, "state").await?;
        h.await?;

        assert_eq!(State::Leader, got.state);
    }

    {
        // wait for members
        let (init, w, tx) = init_wait_test();

        let h = tokio::spawn(async move {
            sleep(Duration::from_millis(10)).await;
            let mut update = init.clone();
            update.membership_config.membership.replace(vec![btreeset![1, 2]]);
            let rst = tx.send(update);
            assert!(rst.is_ok());
        });
        let got = w.members(btreeset![1, 2], "members").await?;
        h.await?;

        assert_eq!(
            btreeset![1, 2],
            got.membership_config.membership.get_ith_config(0).unwrap().clone()
        );
    }

    {
        // wait for next_members
        let (init, w, tx) = init_wait_test();

        let h = tokio::spawn(async move {
            sleep(Duration::from_millis(10)).await;
            let mut update = init.clone();
            update.membership_config.membership.push(btreeset![1, 2]);
            let rst = tx.send(update);
            assert!(rst.is_ok());
        });
        let got = w.next_members(Some(btreeset![1, 2]), "next_members").await?;
        h.await?;

        assert_eq!(
            Some(btreeset![1, 2]),
            got.membership_config.membership.get_ith_config(1).cloned()
        );
    }

    tracing::info!("--- wait for snapshot, Ok");
    {
        let (init, w, tx) = init_wait_test();

        let h = tokio::spawn(async move {
            sleep(Duration::from_millis(10)).await;
            let mut update = init.clone();
            update.snapshot = Some(LogId::new(1, 2));
            let rst = tx.send(update);
            assert!(rst.is_ok());
        });
        let got = w.snapshot(LogId { term: 1, index: 2 }, "snapshot").await?;
        h.await?;

        assert_eq!(Some(LogId::new(1, 2)), got.snapshot);
    }

    tracing::info!("--- wait for snapshot, only index matches");
    {
        let (init, w, tx) = init_wait_test();

        let h = tokio::spawn(async move {
            sleep(Duration::from_millis(10)).await;
            let mut update = init.clone();
            update.snapshot = Some(LogId::new(3, 2));
            let rst = tx.send(update);
            assert!(rst.is_ok());
            // delay otherwise the channel will be closed thus the error is shutdown.
            sleep(Duration::from_millis(200)).await;
        });
        let got = w.snapshot(LogId { term: 1, index: 2 }, "snapshot").await;
        h.await?;
        match got.unwrap_err() {
            WaitError::Timeout(t, _) => {
                assert_eq!(Duration::from_millis(100), t);
            }
            _ => {
                panic!("expect WaitError::Timeout");
            }
        }
    }

    {
        // timeout
        let (_init, w, _tx) = init_wait_test();

        let h = tokio::spawn(async move {
            sleep(Duration::from_millis(200)).await;
        });
        let got = w.state(State::Follower, "timeout").await;
        h.await?;

        match got.unwrap_err() {
            WaitError::Timeout(t, _) => {
                assert_eq!(Duration::from_millis(100), t);
            }
            _ => {
                panic!("expect WaitError::Timeout");
            }
        }
    }

    Ok(())
}

/// Build a initial state for testing of Wait:
/// Returns init metrics, Wait, and the tx to send an updated metrics.
fn init_wait_test() -> (RaftMetrics, Wait, watch::Sender<RaftMetrics>) {
    let init = RaftMetrics {
        running_state: Ok(()),
        id: 0,
        state: State::Learner,
        current_term: 0,
        last_log_index: None,
        last_applied: None,
        current_leader: None,
        membership_config: EffectiveMembership {
            log_id: LogId::default(),
            membership: Membership::new_single(btreeset! {}),
        },

        snapshot: None,
        leader_metrics: None,
    };
    let (tx, rx) = watch::channel(init.clone());
    let w = Wait {
        timeout: Duration::from_millis(100),
        rx,
    };

    (init, w, tx)
}
