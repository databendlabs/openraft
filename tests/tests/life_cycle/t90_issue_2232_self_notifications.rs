use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::EntryPayload;
use openraft::Membership;
use openraft::RaftLogReader;
use openraft::ServerState;
use openraft::Vote;
use openraft::storage::RaftStateMachine;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::WAIT_TIMEOUT;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// A notification channel of capacity one must not deadlock `RaftCore` on its own feedback.
///
/// Persisting the pending vote of the initial election produces two core-generated notifications,
/// `LocalIO` followed by a self `VoteResponse`. With room for one notification, the core used to
/// wait inside the command for capacity that only the core itself could free.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn self_notifications_with_capacity_one() -> Result<()> {
    let config = Config {
        notification_channel_size: Some(1),
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    router.new_raft_node(0).await?;

    let mut log_index = 0;

    tracing::info!(
        log_index,
        "--- initialize node 0, which persists a pending vote and sends itself a VoteResponse"
    );
    {
        let n0 = router.get_raft_handle(&0)?;
        TypeConfig::timeout(WAIT_TIMEOUT, n0.initialize(btreeset! {0})).await??;
        log_index += 1;

        router.wait(&0).state(ServerState::Leader, "node 0 elects itself").await?;
        router.wait(&0).vote(Vote::new_committed(1, 0), "vote committed").await?;
        router.wait(&0).applied_index(Some(log_index), "leader blank log applied").await?;
    }

    tracing::info!(
        log_index,
        "--- check the persisted vote and the applied startup entries"
    );
    {
        let (mut sto, mut sm) = router.get_storage_handle(&0)?;
        assert_eq!(Some(Vote::new_committed(1, 0)), sto.read_vote().await?);

        let entries = sto.try_get_log_entries(..).await?;
        let log_ids: Vec<_> = entries.iter().map(|e| e.log_id).collect();
        assert_eq!(vec![log_id(0, 0, 0), log_id(1, 0, 1)], log_ids);

        let EntryPayload::Membership(membership) = &entries[0].payload else {
            panic!("expect a membership entry at index 0: {}", entries[0]);
        };
        assert_eq!(&Membership::new_with_defaults(vec![btreeset! {0}], []), membership);
        assert!(matches!(entries[1].payload, EntryPayload::Blank), "{}", entries[1]);

        let (last_applied, _) = sm.applied_state().await?;
        assert_eq!(Some(log_id(1, 0, log_index)), last_applied);
    }

    tracing::info!(log_index, "--- write one log through the elected leader");
    {
        let response = TypeConfig::timeout(WAIT_TIMEOUT, router.client_request(0, "foo", 1)).await??;
        log_index += 1;
        assert_eq!(log_id(1, 0, log_index), response.log_id);

        router.wait(&0).applied_index(Some(log_index), "client write applied").await?;

        let (mut sto, mut sm) = router.get_storage_handle(&0)?;
        let entries = sto.try_get_log_entries(log_index..).await?;
        let log_ids: Vec<_> = entries.into_iter().map(|e| e.log_id).collect();
        assert_eq!(vec![log_id(1, 0, log_index)], log_ids);

        let (last_applied, _) = sm.applied_state().await?;
        assert_eq!(Some(log_id(1, 0, log_index)), last_applied);
    }

    tracing::info!(log_index, "--- shut down node 0");
    {
        TypeConfig::timeout(WAIT_TIMEOUT, router.stop_node(0)).await??;
    }

    Ok(())
}

/// A restarted node elects itself again, through an API call rather than initialization, with a
/// notification channel of capacity one.
///
/// With leader restore disabled, the restarted node persists a pending vote for term 2 and sends
/// itself the `VoteResponse` from the command path that handles API messages.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn elect_after_restart_with_capacity_one() -> Result<()> {
    let config = Config {
        notification_channel_size: Some(1),
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        enable_leader_restore: Some(false),
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- bring up a single-node cluster");
    let mut log_index = router.new_cluster([0], []).await?;

    tracing::info!(log_index, "--- write one log, then restart node 0 from its stores");
    {
        let response = router.client_request(0, "foo", 1).await?;
        log_index = response.log_id.index();

        let node = TypeConfig::timeout(WAIT_TIMEOUT, router.stop_node(0)).await??;
        router.new_raft_node_with_sto(0, node.log_store, node.state_machine).await?;
        router.wait(&0).applied_index(Some(log_index), "logs re-applied after restart").await?;
    }

    tracing::info!(log_index, "--- elect node 0 at term 2 through the trigger API");
    {
        let n0 = router.get_raft_handle(&0)?;
        n0.trigger().elect(false).await?;
        log_index += 1;

        router.wait(&0).state(ServerState::Leader, "node 0 elects itself").await?;
        router.wait(&0).vote(Vote::new_committed(2, 0), "vote committed at term 2").await?;
        router.wait(&0).applied_index(Some(log_index), "term 2 blank log applied").await?;

        let (mut sto, _sm) = router.get_storage_handle(&0)?;
        assert_eq!(Some(Vote::new_committed(2, 0)), sto.read_vote().await?);
    }

    tracing::info!(log_index, "--- write one log at term 2");
    {
        let response = TypeConfig::timeout(WAIT_TIMEOUT, router.client_request(0, "foo", 2)).await??;
        log_index += 1;
        assert_eq!(log_id(2, 0, log_index), response.log_id);

        router.wait(&0).applied_index(Some(log_index), "client write applied").await?;
    }

    Ok(())
}

/// A three-node cluster elects a leader, replicates, and applies writes while every node has a
/// notification channel of capacity one.
///
/// Replication tasks, the state machine worker, and storage callbacks all compete with the
/// self-notifications of `RaftCore` for the single slot.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn three_nodes_with_capacity_one() -> Result<()> {
    let config = Config {
        notification_channel_size: Some(1),
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- bring up a three-node cluster");
    let mut log_index = router.new_cluster([0, 1, 2], []).await?;

    tracing::info!(log_index, "--- write 20 logs through node 0");
    {
        let written = TypeConfig::timeout(WAIT_TIMEOUT, router.client_request_many(0, "foo", 20)).await??;
        log_index += 20;
        assert_eq!(Some(log_id(1, 0, log_index)), written.last_log_id);

        for id in [0, 1, 2] {
            router.wait(&id).applied_index(Some(log_index), "writes applied").await?;
        }
    }

    tracing::info!(log_index, "--- check the persisted votes and the applied state");
    {
        for id in [0, 1, 2] {
            let (mut sto, mut sm) = router.get_storage_handle(&id)?;
            assert_eq!(Some(Vote::new_committed(1, 0)), sto.read_vote().await?, "node {}", id);

            let (last_applied, _) = sm.applied_state().await?;
            assert_eq!(Some(log_id(1, 0, log_index)), last_applied, "node {}", id);
        }
    }

    tracing::info!(log_index, "--- shut down every node");
    {
        for id in [0, 1, 2] {
            TypeConfig::timeout(WAIT_TIMEOUT, router.stop_node(id)).await??;
        }
    }

    Ok(())
}
