use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use tokio::sync::watch;
use tokio::time::sleep;

use crate::core::ServerState;
use crate::membership::EffectiveMembership;
use crate::metrics::Wait;
use crate::metrics::WaitError;
use crate::raft_types::LogIdOptionExt;
use crate::LeaderId;
use crate::LogId;
use crate::Membership;
use crate::Node;
use crate::NodeId;
use crate::RaftMetrics;

/// Test wait for different state changes
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn test_wait() -> anyhow::Result<()> {
    {
        // wait for leader
        let (init, w, tx) = init_wait_test::<u64, ()>();

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
        let (init, w, tx) = init_wait_test::<u64, ()>();

        let h = tokio::spawn(async move {
            sleep(Duration::from_millis(10)).await;
            let mut update = init.clone();
            update.last_log_index = Some(3);
            update.last_applied = Some(LogId::new(LeaderId::new(1, 0), 3));
            let rst = tx.send(update);
            assert!(rst.is_ok());
        });
        let got = w.log(Some(3), "log").await?;
        let got_least2 = w.log_at_least(Some(2), "log").await?;
        let got_least3 = w.log_at_least(Some(3), "log").await?;
        let got_least4 = w.log_at_least(Some(4), "log").await;
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
        let (init, w, tx) = init_wait_test::<u64, ()>();

        let h = tokio::spawn(async move {
            sleep(Duration::from_millis(10)).await;
            let mut update = init.clone();
            update.state = ServerState::Leader;
            let rst = tx.send(update);
            assert!(rst.is_ok());
        });
        let got = w.state(ServerState::Leader, "state").await?;
        h.await?;

        assert_eq!(ServerState::Leader, got.state);
    }

    {
        // wait for members
        let (init, w, tx) = init_wait_test::<u64, ()>();

        let h = tokio::spawn(async move {
            sleep(Duration::from_millis(10)).await;
            let mut update = init.clone();
            update.membership_config = Arc::new(EffectiveMembership::new(
                None,
                Membership::new(vec![btreeset! {1,2}], None),
            ));
            let rst = tx.send(update);
            assert!(rst.is_ok());
        });
        let got = w.members(btreeset![1, 2], "members").await?;
        h.await?;

        assert_eq!(
            btreeset![1, 2],
            got.membership_config.membership.get_joint_config().get(0).unwrap().clone()
        );
    }

    tracing::info!("--- wait for snapshot, Ok");
    {
        let (init, w, tx) = init_wait_test::<u64, ()>();

        let h = tokio::spawn(async move {
            sleep(Duration::from_millis(10)).await;
            let mut update = init.clone();
            update.snapshot = Some(LogId::new(LeaderId::new(1, 0), 2));
            let rst = tx.send(update);
            assert!(rst.is_ok());
        });
        let got = w.snapshot(LogId::new(LeaderId::new(1, 0), 2), "snapshot").await?;
        h.await?;

        assert_eq!(Some(LogId::new(LeaderId::new(1, 0), 2)), got.snapshot);
    }

    tracing::info!("--- wait for snapshot, only index matches");
    {
        let (init, w, tx) = init_wait_test::<u64, ()>();

        let h = tokio::spawn(async move {
            sleep(Duration::from_millis(10)).await;
            let mut update = init.clone();
            update.snapshot = Some(LogId::new(LeaderId::new(3, 0), 2));
            let rst = tx.send(update);
            assert!(rst.is_ok());
            // delay otherwise the channel will be closed thus the error is shutdown.
            sleep(Duration::from_millis(200)).await;
        });
        let got = w.snapshot(LogId::new(LeaderId::new(1, 0), 2), "snapshot").await;
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
        let (_init, w, _tx) = init_wait_test::<u64, ()>();

        let h = tokio::spawn(async move {
            sleep(Duration::from_millis(200)).await;
        });
        let got = w.state(ServerState::Follower, "timeout").await;
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

pub(crate) type InitResult<NID, N> = (RaftMetrics<NID, N>, Wait<NID, N>, watch::Sender<RaftMetrics<NID, N>>);

/// Build a initial state for testing of Wait:
/// Returns init metrics, Wait, and the tx to send an updated metrics.
fn init_wait_test<NID, N>() -> InitResult<NID, N>
where
    NID: NodeId,
    N: Node,
{
    let init = RaftMetrics {
        running_state: Ok(()),
        id: NID::default(),
        state: ServerState::Learner,
        current_term: 0,
        last_log_index: None,
        last_applied: None,
        current_leader: None,
        membership_config: Arc::new(EffectiveMembership::new(
            None,
            Membership::new(vec![btreeset! {}], None),
        )),

        snapshot: None,
        replication: None,
    };
    let (tx, rx) = watch::channel(init.clone());
    let w = Wait {
        timeout: Duration::from_millis(100),
        rx,
    };

    (init, w, tx)
}
