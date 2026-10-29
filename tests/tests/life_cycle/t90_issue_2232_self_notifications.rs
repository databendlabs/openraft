use anyhow::Result;
use maplit::btreeset;
use maplit::hashmap;
use openraft::Config;
use openraft::RaftLogReader;
use openraft::Vote;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::ClientResponse;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::WAIT_TIMEOUT;
use crate::fixtures::WriteSummary;
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

        router.wait(&0).vote(Vote::new_committed(1, 0), "vote committed").await?;
        router.wait(&0).applied_index(Some(log_index), "leader blank log applied").await?;
    }

    tracing::info!(log_index, "--- check the persisted vote");
    {
        let (mut sto, _sm) = router.get_storage_handle(&0)?;
        let persisted_vote = sto.read_vote().await?;
        let expected_vote = Some(Vote::new_committed(1, 0));
        assert_eq!(expected_vote, persisted_vote);
    }

    tracing::info!(log_index, "--- write one log through the elected leader");
    {
        let response = TypeConfig::timeout(WAIT_TIMEOUT, router.client_request(0, "foo", 1)).await??;
        log_index += 1;
        let expected_log_id = log_id(1, 0, log_index);
        assert_eq!(expected_log_id, response.log_id);
        let expected_response = ClientResponse(None);
        assert_eq!(expected_response, response.data);

        router.wait(&0).applied_index(Some(log_index), "client write applied").await?;

        let (_sto, sm) = router.get_storage_handle(&0)?;
        let state = sm.get_state_machine().await;
        let expected_status = hashmap! {"foo".to_string() => "request-1".to_string()};
        assert_eq!(expected_status, state.client_status);
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
        let expected_response = ClientResponse(None);
        assert_eq!(expected_response, response.data);

        let node = TypeConfig::timeout(WAIT_TIMEOUT, router.stop_node(0)).await??;
        router.new_raft_node_with_sto(0, node.log_store, node.state_machine).await?;
        router.wait(&0).applied_index(Some(log_index), "logs re-applied after restart").await?;
    }

    tracing::info!(log_index, "--- elect node 0 at term 2 through the trigger API");
    {
        let n0 = router.get_raft_handle(&0)?;
        n0.trigger().elect(false).await?;
        log_index += 1;

        router.wait(&0).vote(Vote::new_committed(2, 0), "vote committed at term 2").await?;
        router.wait(&0).applied_index(Some(log_index), "term 2 blank log applied").await?;

        let (mut sto, _sm) = router.get_storage_handle(&0)?;
        let persisted_vote = sto.read_vote().await?;
        let expected_vote = Some(Vote::new_committed(2, 0));
        assert_eq!(expected_vote, persisted_vote);
    }

    tracing::info!(log_index, "--- write one log at term 2");
    {
        let response = TypeConfig::timeout(WAIT_TIMEOUT, router.client_request(0, "foo", 2)).await??;
        log_index += 1;
        let expected_log_id = log_id(2, 0, log_index);
        assert_eq!(expected_log_id, response.log_id);
        let expected_response = ClientResponse(Some("request-1".to_string()));
        assert_eq!(expected_response, response.data);

        router.wait(&0).applied_index(Some(log_index), "client write applied").await?;

        let (_sto, sm) = router.get_storage_handle(&0)?;
        let state = sm.get_state_machine().await;
        let expected_status = hashmap! {"foo".to_string() => "request-2".to_string()};
        assert_eq!(expected_status, state.client_status);
    }

    tracing::info!(log_index, "--- shut down node 0 after the second election");
    {
        TypeConfig::timeout(WAIT_TIMEOUT, router.stop_node(0)).await??;
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
        let expected_written = WriteSummary {
            count: 20,
            last_log_id: Some(log_id(1, 0, log_index)),
        };
        assert_eq!(expected_written, written);

        for id in [0, 1, 2] {
            router.wait(&id).applied_index(Some(log_index), "writes applied").await?;
        }
    }

    tracing::info!(log_index, "--- check the persisted votes and the applied state");
    {
        for id in [0, 1, 2] {
            let (mut sto, sm) = router.get_storage_handle(&id)?;
            let persisted_vote = sto.read_vote().await?;
            let expected_vote = Some(Vote::new_committed(1, 0));
            assert_eq!(expected_vote, persisted_vote, "node {}", id);

            let state = sm.get_state_machine().await;
            let expected_status = hashmap! {"foo".to_string() => "request-19".to_string()};
            assert_eq!(expected_status, state.client_status, "node {}", id);
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
