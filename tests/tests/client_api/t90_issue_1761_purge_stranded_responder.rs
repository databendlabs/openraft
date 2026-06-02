use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::Vote;
use openraft::async_runtime::OneshotSender;
use openraft::errors::ClientWriteError;
use openraft::errors::ForwardToLeader;
use openraft::errors::RaftError;
use openraft::raft::AppendEntriesRequest;
use openraft::testing::blank_ent;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::ClientRequest;
use openraft_memstore::IntoMemClientRequest;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// Regression test for <https://github.com/databendlabs/openraft/issues/1761>.
///
/// A deposed leader's pending client responder must not survive a snapshot install that purges past
/// it.
///
/// A leader appends a client log but loses leadership before it commits, so the responder lingers
/// in `client_responders`. The node then lags and installs a snapshot whose last log index is
/// beyond its local log. `install_full_snapshot` skips the conflict-truncate in that case (so
/// `TruncateLog` does not drain the responder) and removes the entry via `PurgeLog`. Before the
/// fix, `PurgeLog` did not drain responders, leaving one stranded below the purge boundary; the
/// next apply then called `get_log_id(index).unwrap()` on a purged index and panicked the
/// `RaftCore` task.
///
/// After the fix, `PurgeLog` drains and fails the responders it covers, so the pending write
/// resolves with `ForwardToLeader` and the node keeps applying.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn write_then_superseded_by_snapshot_install() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_tick: false,
            enable_heartbeat: false,
            enable_elect: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing 3-node cluster; node 0 is leader");
    let log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    tracing::info!(log_index, "--- isolate node 0 so its client write can never commit");
    router.set_unreachable(0, true);

    let (tx, rx) = TypeConfig::oneshot();

    tracing::info!(
        log_index,
        "--- node 0 (still leader) accepts a client write that will never commit"
    );
    {
        let n0 = router.get_raft_handle(&0)?;
        TypeConfig::spawn(async move {
            let res = n0.client_write(ClientRequest::make_request("cli", 1)).await;
            tx.send(res).unwrap();
        });
    }

    // Wait for the log to be appended on node 0 and its responder installed at index log_index+1.
    TypeConfig::sleep(Duration::from_millis(500)).await;

    tracing::info!(
        log_index,
        "--- elect node 1 on the {{1,2}} side, leaving node 0's responder stranded"
    );
    {
        let n1 = router.get_raft_handle(&1)?;
        n1.trigger().elect().await?;
        // node 1 becomes leader at term 2 and commits its blank log at index log_index+1.
        router.wait(&1, timeout()).applied_index(Some(log_index + 1), "node 1 leader blank").await?;
    }

    tracing::info!(log_index, "--- advance node 1 well past node 0, then snapshot it");
    let snap_index = log_index + 1 + router.client_request_many(1, "foo", 10).await?;
    router.wait(&1, timeout()).applied_index(Some(snap_index), "node 1 advanced").await?;
    router.wait(&2, timeout()).applied_index(Some(snap_index), "node 2 advanced").await?;

    let snap = {
        let n1 = router.get_raft_handle(&1)?;
        n1.trigger().snapshot().await?;
        router.wait(&1, timeout()).snapshot(log_id(2, 1, snap_index), "node 1 snapshot").await?;
        n1.get_snapshot().await?.unwrap()
    };

    tracing::info!(
        snap_index,
        "--- install node 1's snapshot on node 0; its index is beyond node 0's local log"
    );
    {
        let n0 = router.get_raft_handle(&0)?;
        n0.install_full_snapshot(Vote::new_committed(2, 1), snap).await?;
    }

    tracing::info!("--- drive one more apply on node 0; before the fix this panics on the stranded responder");
    {
        let n0 = router.get_raft_handle(&0)?;
        n0.append_entries(AppendEntriesRequest {
            vote: Vote::new_committed(2, 1),
            prev_log_id: Some(log_id(2, 1, snap_index)),
            entries: vec![blank_ent::<TypeConfig>(2, 1, snap_index + 1)],
            leader_commit: Some(log_id(2, 1, snap_index + 1)),
        })
        .await?;
    }

    tracing::info!("--- the pending write must resolve with ForwardToLeader, not panic the core");
    let write_res = rx.await?;
    let raft_err = write_res.unwrap_err();
    assert_eq!(
        raft_err,
        RaftError::APIError(ClientWriteError::ForwardToLeader(ForwardToLeader {
            leader_id: Some(1),
            leader_node: Some(()),
        }))
    );

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
