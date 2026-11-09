use std::sync::Arc;
use std::time::Duration;

use futures_util::poll;
use maplit::btreemap;
use maplit::btreeset;

use super::*;
use crate::Membership;
use crate::async_runtime::WatchSender;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::type_config::alias::StoredMembershipOf;

#[test]
fn test_replication_wait() {
    UTConfig::<()>::run(async {
        let mut metrics = RaftMetrics::<UTConfig>::new_initial(0);
        metrics.state = ServerState::Leader;
        metrics.last_log_index = Some(10);
        metrics.membership_config = Arc::new(StoredMembershipOf::<UTConfig>::new(
            Some(log_id(1, 0, 5)),
            Membership::new_with_defaults(vec![btreeset! {0}], [1]),
        ));
        let (tx, rx) = UTConfig::<()>::watch_channel(metrics.clone());
        let wait = Wait::<UTConfig> { timeout: None, rx };
        let mut pending = Box::pin(wait.replication(1, Some(8), 3));

        tracing::info!("existing target with missing replication metrics must remain pending");
        {
            assert!(poll!(&mut pending).is_pending());
            metrics.replication = Some(btreemap! {});
            tx.send(metrics.clone())?;
            assert!(poll!(&mut pending).is_pending());
        }

        tracing::info!("small lag alone must not satisfy the minimum matched index of 8");
        {
            metrics.replication = Some(btreemap! {1 => Some(log_id(1, 0, 7))});
            tx.send(metrics.clone())?;
            assert!(poll!(&mut pending).is_pending());
        }

        tracing::info!("minimum index alone must not satisfy lag as the leader advances to 12");
        {
            metrics.last_log_index = Some(12);
            metrics.replication = Some(btreemap! {1 => Some(log_id(1, 0, 8))});
            tx.send(metrics.clone())?;
            assert!(poll!(&mut pending).is_pending());
        }

        tracing::info!("complete at the inclusive lag boundary: matched=9, last=12, max_lag=3");
        {
            metrics.replication = Some(btreemap! {1 => Some(log_id(1, 0, 9))});
            tx.send(metrics.clone())?;
            let got = pending.await?;
            assert_eq!(metrics, got);
        }

        tracing::info!("no minimum and no logs may succeed; self uses the leader's own index");
        {
            metrics.last_log_index = None;
            metrics.replication = Some(btreemap! {1 => None});
            tx.send(metrics.clone())?;
            assert_eq!(metrics, wait.replication(1, None, 0).await?);
            assert_eq!(metrics, wait.replication(0, None, 0).await?);

            let mut pending = Box::pin(wait.replication(1, Some(0), u64::MAX));
            assert!(poll!(&mut pending).is_pending());
            metrics.last_log_index = Some(0);
            tx.send(metrics.clone())?;
            assert!(poll!(&mut pending).is_pending());
            assert_eq!(metrics, wait.replication(1, None, 1).await?);
            metrics.replication = Some(btreemap! {1 => Some(log_id(1, 0, 0))});
            tx.send(metrics.clone())?;
            assert_eq!(metrics, pending.await?);
            assert_eq!(metrics, wait.replication(0, Some(0), 0).await?);
        }

        tracing::info!("zero lag requires matching the leader's latest index");
        {
            metrics.last_log_index = Some(1);
            tx.send(metrics.clone())?;
            let mut pending = Box::pin(wait.replication(1, Some(0), 0));
            assert!(poll!(&mut pending).is_pending());
            metrics.replication = Some(btreemap! {1 => Some(log_id(1, 0, 1))});
            tx.send(metrics.clone())?;
            assert_eq!(metrics, pending.await?);
        }

        Ok::<(), anyhow::Error>(())
    })
    .unwrap();
}

#[test]
fn test_replication_wait_metrics_freshness() {
    UTConfig::<()>::run(async {
        let mut metrics = RaftMetrics::<UTConfig>::new_initial(0);
        metrics.state = ServerState::Leader;
        let (tx, rx) = UTConfig::<()>::watch_channel(metrics.clone());
        let wait = Wait::<UTConfig> { timeout: None, rx };
        let mut pending = Box::pin(wait.replication(1, Some(5), u64::MAX));

        tracing::info!("metrics without the membership entry at index 5 must not imply removal");
        {
            assert!(poll!(&mut pending).is_pending());
            metrics.last_log_index = Some(4);
            tx.send(metrics.clone())?;
            assert!(poll!(&mut pending).is_pending());
        }

        tracing::info!("fresh metrics include the learner, but replication must also reach index 5");
        {
            metrics.last_log_index = Some(5);
            metrics.membership_config = Arc::new(StoredMembershipOf::<UTConfig>::new(
                Some(log_id(1, 0, 5)),
                Membership::new_with_defaults(vec![btreeset! {0}], [1]),
            ));
            metrics.replication = Some(btreemap! {1 => Some(log_id(1, 0, 4))});
            tx.send(metrics.clone())?;
            assert!(poll!(&mut pending).is_pending());
            metrics.replication = Some(btreemap! {1 => Some(log_id(1, 0, 5))});
            tx.send(metrics.clone())?;
            assert_eq!(metrics, pending.await?);
        }

        tracing::info!("a missing target becomes an error only once metrics reach the requested index 6");
        {
            let mut pending = Box::pin(wait.replication(1, Some(6), 0));
            metrics.membership_config = Arc::new(StoredMembershipOf::<UTConfig>::default());
            tx.send(metrics.clone())?;
            assert!(poll!(&mut pending).is_pending());
            metrics.last_log_index = Some(6);
            tx.send(metrics.clone())?;
            assert_eq!(ReplicationWaitError::NodeNotFound, pending.await.unwrap_err());
        }

        tracing::info!("leadership loss must end a wait even before metrics reach the requested index 7");
        {
            let mut pending = Box::pin(wait.replication(1, Some(7), 0));
            assert!(poll!(&mut pending).is_pending());
            metrics.state = ServerState::Follower;
            tx.send(metrics)?;
            assert_eq!(ReplicationWaitError::NotLeader, pending.await.unwrap_err());
        }

        Ok::<(), anyhow::Error>(())
    })
    .unwrap();
}

#[test]
fn test_replication_wait_errors() {
    UTConfig::<()>::run(async {
        let mut metrics = RaftMetrics::<UTConfig>::new_initial(0);
        metrics.state = ServerState::Leader;
        metrics.last_log_index = Some(10);
        metrics.replication = Some(btreemap! {1 => Some(log_id(1, 0, 5))});
        metrics.membership_config = Arc::new(StoredMembershipOf::<UTConfig>::new(
            Some(log_id(1, 0, 5)),
            Membership::new_with_defaults(vec![btreeset! {0}], [1]),
        ));
        let (tx, rx) = UTConfig::<()>::watch_channel(metrics.clone());
        let mut wait = Wait::<UTConfig> { timeout: None, rx };

        tracing::info!("removing the target interrupts a pending replication wait");
        {
            let mut pending = Box::pin(wait.replication(1, Some(10), 0));
            assert!(poll!(&mut pending).is_pending());
            let mut removed = metrics.clone();
            removed.membership_config = Arc::new(StoredMembershipOf::<UTConfig>::default());
            tx.send(removed)?;
            assert_eq!(ReplicationWaitError::NodeNotFound, pending.await.unwrap_err());
            assert_eq!(
                ReplicationWaitError::NodeNotFound,
                wait.replication(1, None, 10).await.unwrap_err()
            );
        }

        tracing::info!("losing leadership interrupts the wait rather than reporting catch-up");
        {
            tx.send(metrics.clone())?;
            let mut pending = Box::pin(wait.replication(1, Some(10), 0));
            assert!(poll!(&mut pending).is_pending());
            let mut follower = metrics.clone();
            follower.state = ServerState::Follower;
            follower.replication = None;
            tx.send(follower)?;
            assert_eq!(ReplicationWaitError::NotLeader, pending.await.unwrap_err());
            assert_eq!(
                ReplicationWaitError::NotLeader,
                wait.replication(0, None, 0).await.unwrap_err()
            );
        }

        tracing::info!("timeout is reported without changing the replication condition");
        {
            tx.send(metrics.clone())?;
            wait.timeout = Some(Duration::ZERO);
            let err = wait.replication(1, Some(10), 0).await.unwrap_err();
            assert!(matches!(
                err,
                ReplicationWaitError::Wait(WaitError::Timeout(Duration::ZERO, _))
            ));
        }

        tracing::info!("shutdown interrupts a wait with no timeout");
        {
            wait.timeout = None;
            let mut pending = Box::pin(wait.replication(1, Some(10), 0));
            assert!(poll!(&mut pending).is_pending());
            drop(tx);
            assert_eq!(
                ReplicationWaitError::Wait(WaitError::ShuttingDown),
                pending.await.unwrap_err()
            );
        }

        Ok::<(), anyhow::Error>(())
    })
    .unwrap();
}
