use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::errors::NetworkError;
use openraft::errors::RPCError;
use openraft_memstore::ClientRequest;
use openraft_memstore::IntoMemClientRequest;
use openraft_memstore::MemNodeId;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::RpcCall;
use crate::fixtures::RpcResponseCall;
use crate::fixtures::log_id;
use crate::fixtures::rpc_kind::RpcKind;
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
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- bring up a three-voter cluster led by node 0");
    let log_index = router.new_cluster(btreeset! {0, 1, 2}, btreeset! {}).await?;

    tracing::info!(log_index, "--- lose every AppendEntries reply to node 0");
    {
        lose_replies_to(&router, &[RpcKind::AppendEntries], 0).await;
    }

    tracing::info!(
        log_index,
        "--- write an entry that nodes 1 and 2 accept but node 0 cannot commit"
    );
    {
        let n0 = router.get_raft_handle(&0)?;
        n0.client_write_ff(ClientRequest::make_request("foo", 1), None).await?;

        for id in [1, 2] {
            router
                .wait_with_timeout(&id, window())
                .log_index(Some(log_index + 1), "the entry reaches the follower")
                .await?;
        }
    }

    tracing::info!(
        log_index,
        "--- nodes 1 and 2 elect a new leader while node 0 keeps quiet"
    );
    {
        router
            .wait_with_timeout(&1, window())
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
    snapshot_retries_allow_election(false).await
}

/// A quiet period holds back final-chunk retries so the followers can elect a leader.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn quiet_period_holds_back_chunked_snapshot_retries() -> Result<()> {
    snapshot_retries_allow_election(true).await
}

async fn snapshot_retries_allow_election(chunked: bool) -> Result<()> {
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
    let mut builder = RaftRouter::builder(config)?;
    if chunked {
        builder = builder.chunked_snapshots();
    }
    let mut router = builder.build();

    tracing::info!("--- bring up a three-voter cluster led by node 0");
    let mut log_index = router.new_cluster(btreeset! {0, 1, 2}, btreeset! {}).await?;

    tracing::info!(log_index, "--- leave node 2 behind node 0's purged log");
    {
        lose_requests_to(&router, &[RpcKind::AppendEntries], 2).await;
        lose_requests_to(&router, &SNAPSHOT, 2).await;

        log_index += router.client_request_many(0, "0", 10).await?.count;

        let n0 = router.get_raft_handle(&0)?;
        n0.trigger().snapshot().await?;
        n0.wait(timeout()).purged(Some(log_id(1, 0, log_index)), "node 0 purges its snapshot logs").await?;
    }

    tracing::info!(
        log_index,
        "--- reconnect node 2, losing every reply and Vote request to node 0"
    );
    {
        lose_replies_to(&router, &[RpcKind::AppendEntries], 0).await;
        lose_replies_to(&router, &SNAPSHOT, 0).await;
        lose_requests_to(&router, &VOTE, 0).await;
        for kind in [RpcKind::AppendEntries].into_iter().chain(SNAPSHOT) {
            router.clear_rpc_pre_hook(kind).await;
        }

        router
            .wait_with_timeout(&2, window())
            .snapshot(log_id(1, 0, log_index), "node 2 installs the snapshot")
            .await?;

        if chunked {
            let chunks = router.snapshot_chunks();
            let chunk = &chunks[0];
            assert_eq!(0, chunk.offset);
            assert!(chunk.done);
        }
    }

    tracing::info!(
        log_index,
        "--- nodes 1 and 2 elect a new leader while node 0 keeps quiet"
    );
    {
        router
            .wait_with_timeout(&1, window())
            .metrics(
                |metrics| metrics.current_leader.is_some_and(|leader| leader != 0),
                "node 1 follows a new leader",
            )
            .await?;
    }

    Ok(())
}

/// Every snapshot RPC, chunked or whole.
const SNAPSHOT: [RpcKind; 2] = [RpcKind::InstallSnapshot, RpcKind::InstallFullSnapshot];

/// Both election phases.
const VOTE: [RpcKind; 2] = [RpcKind::PreVote, RpcKind::Vote];

/// Lose every reply to `leader` of the RPCs in `kinds`, after the request has reached its target.
async fn lose_replies_to(router: &RaftRouter, kinds: &[RpcKind], leader: MemNodeId) {
    for &kind in kinds {
        router
            .set_rpc_post_hook(kind, move |call: RpcResponseCall| {
                let res = if call.from == leader {
                    let err =
                        NetworkError::<TypeConfig>::from_string(format!("reply lost: {}->{}", call.to, call.from));
                    Err(RPCError::Network(err))
                } else {
                    Ok(())
                };
                futures::future::ready(res)
            })
            .await;
    }
}

/// Lose every request to `target` of the RPCs in `kinds`.
async fn lose_requests_to(router: &RaftRouter, kinds: &[RpcKind], target: MemNodeId) {
    for &kind in kinds {
        router
            .set_rpc_pre_hook(kind, move |call: RpcCall| {
                let res = if call.to == target {
                    let err =
                        NetworkError::<TypeConfig>::from_string(format!("request lost: {}->{}", call.from, call.to));
                    Err(RPCError::Network(err))
                } else {
                    Ok(())
                };
                futures::future::ready(res)
            })
            .await;
    }
}

fn window() -> Duration {
    Duration::from_millis(2_000)
}

fn timeout() -> Option<Duration> {
    Some(window())
}
