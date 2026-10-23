use std::time::Duration;

use maplit::btreeset;
use maplit::hashmap;
use openraft::Config;
use openraft::Entry;
use openraft::EntryPayload;
use openraft::RaftLogReader;
use openraft::ServerState;
use openraft::Vote;
use openraft::async_runtime::MpscReceiver;
use openraft::async_runtime::MpscSender;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::storage::RaftLogStorageExt;
use openraft::testing::blank_ent;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::ClientRequest;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::WAIT_TIMEOUT;
use crate::fixtures::expect_timeout;
use crate::fixtures::log_id;
use crate::fixtures::rpc_kind::RpcKind;
use crate::fixtures::rpc_request::RpcRequest;
use crate::fixtures::rpc_response::RpcResponse;
use crate::fixtures::ut_harness;

/// A restarted leader must not assign the log id of a lost entry to a different entry.
///
/// Leader node-0 replicated entry X at index 6 to node-1, then crashed before its own flush
/// finished, so its durable log ends at index 5. If node-0 restored leadership in term 1, its next
/// entry would get X's log id. node-1 would see an equal log id and keep X. Then node-1 applies X,
/// while node-0 and node-2 apply the new entry.
///
/// node-0 must win a term-2 election first. Then index 6 gets a new log id, and replication
/// replaces X on node-1.
///
/// See: <https://github.com/databendlabs/openraft/issues/2167>
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn issue_2167_leader_restore_reuses_log_id() -> anyhow::Result<()> {
    let config = Config {
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        enable_leader_restore: Some(true),
        ..Default::default()
    };

    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- bring up a 3-node cluster; node-0 is the leader at term 1");
    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    tracing::info!(
        log_index,
        "--- stop every node; a restarted node-2 has no leader lease, so it grants a vote at once"
    );
    let n0 = router.stop_node(0).await?;
    let n1 = router.stop_node(1).await?;
    let n2 = router.stop_node(2).await?;
    let mut ls1 = n1.log_store;

    let x_index = log_index + 1;

    tracing::info!(
        x_index,
        "--- only node-1 holds uncommitted entry X; node-0 lost it in the crash"
    );
    {
        ls1.blocking_append([Entry {
            log_id: log_id(1, 0, x_index),
            payload: EntryPayload::Normal(ClientRequest {
                client: "x".to_string(),
                serial: 0,
                status: "lost".to_string(),
            }),
        }])
        .await?;
    }

    tracing::info!(log_index, "--- restart node-2, then node-0; node-1 stays offline");
    {
        router.new_raft_node_with_sto(2, n2.log_store, n2.state_machine).await?;
        router.new_raft_node_with_sto(0, n0.log_store, n0.state_machine).await?;
    }

    tracing::info!(log_index, "--- node-0 must win a term-2 election before it proposes");
    {
        router.wait(&0).vote(Vote::new_committed(2, 0), "node-0 elected at term 2").await?;

        // The term-2 blank log takes the index of X.
        log_index += 1;
        router.wait(&0).applied_index(Some(log_index), "term-2 blank log committed").await?;
    }

    tracing::info!(log_index, "--- write one entry, committed by node-0 and node-2");
    {
        router.client_request(0, "foo", 0).await?;
        log_index += 1;

        router.wait(&2).applied_index(Some(log_index), "node-2 applies the write").await?;
    }

    tracing::info!(log_index, "--- restart node-1; replication must replace X");
    {
        router.new_raft_node_with_sto(1, ls1, n1.state_machine).await?;
        router.wait(&1).applied_index(Some(log_index), "node-1 catches up").await?;
    }

    tracing::info!(log_index, "--- every node holds the same log suffix and state machine");
    {
        let want_log = vec![
            (log_id(2, 0, x_index), "blank".to_string()),
            (
                log_id(2, 0, x_index + 1),
                "normal:ClientRequest{client:foo, serial:0, status:request-0}".to_string(),
            ),
        ];
        let want_client_status = hashmap! {"foo".to_string() => "request-0".to_string()};

        for id in [0, 1, 2] {
            let (mut ls, sm) = router.get_storage_handle(&id)?;

            let entries = ls.try_get_log_entries(x_index..).await?;
            let got_log: Vec<_> = entries.into_iter().map(|e| (e.log_id, e.payload.to_string())).collect();
            assert_eq!(want_log, got_log, "node-{} log from index {}", id, x_index);

            let sm = sm.get_state_machine().await;
            assert_eq!(
                Some(log_id(2, 0, x_index + 1)),
                sm.last_applied_log,
                "node-{} last applied",
                id
            );
            assert_eq!(want_client_status, sm.client_status, "node-{} client status", id);
        }
    }

    Ok(())
}

/// A required voter with a newer log must reject the restarted leader's startup campaign.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn leader_restore_rejected_by_newer_log() -> anyhow::Result<()> {
    let config = Config {
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        enable_leader_restore: Some(true),
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- bring up two voters, so node-0 needs node-1 to win an election");
    let log_index = router.new_cluster([0, 1], []).await?;

    tracing::info!(
        log_index,
        "--- stop both nodes and preserve node-0's shorter durable log"
    );
    let (n0, mut n1, before) = {
        let mut n0 = router.stop_node(0).await?;
        let n1 = router.stop_node(1).await?;
        let entries = n0.log_store.try_get_log_entries(..).await?;
        let before = bincode::serialize(&entries)?;
        (n0, n1, before)
    };
    let x_index = log_index + 1;

    tracing::info!(x_index, "--- only node-1 retains an extra uncommitted entry");
    {
        let entry = blank_ent::<TypeConfig>(1, 0, x_index);
        n1.log_store.blocking_append([entry]).await?;
    }

    let (vote_tx, mut vote_rx) = TypeConfig::mpsc(1);

    tracing::info!(
        x_index,
        "--- observe the startup vote request and the required voter's response"
    );
    {
        router
            .set_rpc_post_hook(RpcKind::Vote, move |call| {
                let vote_tx = vote_tx.clone();
                async move {
                    assert_eq!(0, call.from);
                    assert_eq!(1, call.to);
                    let RpcRequest::Vote(request) = call.request else {
                        panic!("expected a vote request");
                    };
                    let RpcResponse::Vote(response) = call.response else {
                        panic!("expected a vote response");
                    };
                    let result = vote_tx.send((request, response)).await;
                    result.expect("vote observation receiver is alive");
                    Ok(())
                }
            })
            .await;
    }

    tracing::info!(
        log_index,
        x_index,
        "--- restart node-1 without a lease, then let node-0 campaign"
    );
    {
        router.new_raft_node_with_sto(1, n1.log_store, n1.state_machine).await?;
        router.new_raft_node_with_sto(0, n0.log_store, n0.state_machine).await?;

        let receive = vote_rx.recv();
        let exchange = TypeConfig::timeout(WAIT_TIMEOUT, receive).await?;
        let (request, response) = exchange.expect("startup campaign produces a vote response");
        let vote = Vote::new(2, 0);
        let last_log_id = Some(log_id(1, 0, log_index));
        let want = VoteRequest::new(vote, last_log_id);
        assert_eq!(want, request);
        let vote = Vote::new_committed(1, 0);
        let last_log_id = Some(log_id(1, 0, x_index));
        let want = VoteResponse::new(vote, last_log_id, false);
        assert_eq!(want, response);
    }

    tracing::info!(
        log_index,
        "--- the rejected campaign must not establish leadership or append a blank"
    );
    {
        let wait = router.wait_with_timeout(&0, Duration::from_millis(100));
        let result = wait.state(ServerState::Leader, "a required voter rejected the campaign").await;
        expect_timeout(result)?;

        let metrics = router.get_metrics(&0)?;
        assert_eq!(ServerState::Candidate, metrics.state);
        let want = Vote::new(2, 0);
        assert_eq!(want, metrics.vote);
        assert_eq!(None, metrics.cluster_committed);

        let (mut ls, _) = router.get_storage_handle(&0)?;
        let entries = ls.try_get_log_entries(..).await?;
        let after = bincode::serialize(&entries)?;
        assert_eq!(before, after);
    }

    Ok(())
}
