use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::RPCTypes;
use openraft::errors::NetworkError;
use openraft::errors::RPCError;
use openraft_memstore::ClientRequest;
use openraft_memstore::IntoMemClientRequest;
use openraft_memstore::MemNodeId;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// A leader without a quorum holds back replication retries, not just heartbeats, during a
/// quorum-loss quiet period, so that its followers' leases expire and they can elect a leader.
///
/// Node 0's AppendEntries reach nodes 1 and 2, but every reply is lost. Node 0 keeps retrying an
/// entry it cannot commit, every 50 ms, well within a follower's lease. A follower that accepts a
/// retry renews its lease of node 0, as for a heartbeat, so if the retries went on while node 0
/// suppressed heartbeats, neither follower would ever campaign.
///
/// The interval leaves room for an election in the first quiet period, and the wait ends before
/// the second one starts.
///
/// See: <https://github.com/databendlabs/openraft/issues/2166>
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn quiet_period_holds_back_replication_retries() -> Result<()> {
    let config = Config {
        election_timeout_min: 150,
        election_timeout_max: 300,
        quorum_loss_probe_interval: Some(1_000),
        backoff: "50ms 50ms 50ms".to_string(),
        ..Default::default()
    }
    .validate()?;
    let mut router = RaftRouter::new(Arc::new(config));

    tracing::info!("--- bring up a three-voter cluster led by node 0");
    let log_index = router.new_cluster(btreeset! {0, 1, 2}, btreeset! {}).await?;

    tracing::info!(log_index, "--- lose every AppendEntries reply to node 0");
    {
        lose_replies_to(&router, RPCTypes::AppendEntries, 0).await;
    }

    tracing::info!(
        log_index,
        "--- write an entry that nodes 1 and 2 accept but node 0 cannot commit"
    );
    {
        let n0 = router.get_raft_handle(&0)?;
        n0.client_write_ff(ClientRequest::make_request("foo", 1), None).await?;

        for id in [1, 2] {
            router.wait(&id, timeout()).log_index(Some(log_index + 1), "the entry reaches the follower").await?;
        }
    }

    tracing::info!(
        log_index,
        "--- nodes 1 and 2 elect a new leader while node 0 keeps quiet"
    );
    {
        router
            .wait(&1, timeout())
            .metrics(
                |metrics| metrics.current_leader.is_some_and(|leader| leader != 0),
                "node 1 follows a new leader",
            )
            .await?;
    }

    Ok(())
}

/// A leader without a quorum holds back snapshot retries during a quorum-loss quiet period.
///
/// Node 2 is behind node 0's purged log, and node 0 resends it the snapshot every 50 ms, because
/// every reply is lost. Node 1's Vote requests to node 0 are lost too, so node 1 needs node 2's
/// vote, which node 2 refuses for as long as the resends renew its lease of node 0.
///
/// See: <https://github.com/databendlabs/openraft/issues/2166>
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn quiet_period_holds_back_snapshot_retries() -> Result<()> {
    let config = Config {
        election_timeout_min: 150,
        election_timeout_max: 300,
        // Node 2 campaigns in vain while it lags; Pre-Vote keeps its term from rising.
        enable_pre_vote: Some(true),
        quorum_loss_probe_interval: Some(1_000),
        backoff: "50ms 50ms 50ms".to_string(),
        max_in_snapshot_log_to_keep: 0,
        purge_batch_size: 1,
        ..Default::default()
    }
    .validate()?;
    let mut router = RaftRouter::new(Arc::new(config));

    tracing::info!("--- bring up a three-voter cluster led by node 0");
    let mut log_index = router.new_cluster(btreeset! {0, 1, 2}, btreeset! {}).await?;

    tracing::info!(log_index, "--- leave node 2 behind node 0's purged log");
    {
        lose_requests_to(&router, RPCTypes::AppendEntries, 2).await;
        lose_requests_to(&router, RPCTypes::InstallSnapshot, 2).await;

        log_index += router.client_request_many(0, "0", 10).await?;

        let n0 = router.get_raft_handle(&0)?;
        n0.trigger().snapshot().await?;
        n0.wait(timeout()).purged(Some(log_id(1, 0, log_index)), "node 0 purges its snapshot logs").await?;
    }

    tracing::info!(
        log_index,
        "--- reconnect node 2, losing every reply and Vote request to node 0"
    );
    {
        lose_replies_to(&router, RPCTypes::AppendEntries, 0).await;
        lose_replies_to(&router, RPCTypes::InstallSnapshot, 0).await;
        lose_requests_to(&router, RPCTypes::Vote, 0).await;
        router.rpc_pre_hook(RPCTypes::AppendEntries, None).await;
        router.rpc_pre_hook(RPCTypes::InstallSnapshot, None).await;

        router.wait(&2, timeout()).snapshot(log_id(1, 0, log_index), "node 2 installs the snapshot").await?;
    }

    tracing::info!(
        log_index,
        "--- nodes 1 and 2 elect a new leader while node 0 keeps quiet"
    );
    {
        router
            .wait(&1, timeout())
            .metrics(
                |metrics| metrics.current_leader.is_some_and(|leader| leader != 0),
                "node 1 follows a new leader",
            )
            .await?;
    }

    Ok(())
}

/// Lose every `rpc_type` reply to `leader`, after the request has reached its target.
async fn lose_replies_to(router: &RaftRouter, rpc_type: RPCTypes, leader: MemNodeId) {
    router
        .set_rpc_post_hook(rpc_type, move |_router, _req, _resp, from, to| {
            let res = if from == leader {
                let err = NetworkError::<TypeConfig>::from_string(format!("reply lost: {}->{}", to, from));
                Err(RPCError::Network(err))
            } else {
                Ok(())
            };
            Box::pin(futures::future::ready(res))
        })
        .await;
}

/// Lose every `rpc_type` request to `target`.
async fn lose_requests_to(router: &RaftRouter, rpc_type: RPCTypes, target: MemNodeId) {
    router
        .set_rpc_pre_hook(rpc_type, move |_router, _req, from, to| {
            let res = if to == target {
                let err = NetworkError::<TypeConfig>::from_string(format!("request lost: {}->{}", from, to));
                Err(RPCError::Network(err))
            } else {
                Ok(())
            };
            Box::pin(futures::future::ready(res))
        })
        .await;
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2_000))
}
