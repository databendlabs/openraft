use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::entry::RaftEntry;
use openraft::errors::LinearizableReadError;
use openraft::errors::RaftError;
use openraft::raft::linearizable_read::LinearizerOption;
use openraft::type_config::OneshotSender;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::WAIT_TIMEOUT;
use crate::fixtures::rpc_kind::RpcKind;
use crate::fixtures::rpc_request::RpcRequest;
use crate::fixtures::ut_harness;

/// A delayed data response cannot credit a newer commit-only request at the same log boundary.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn pipelined_ack_does_not_confirm_read() -> Result<()> {
    let config = Config {
        election_timeout_min: 5_000,
        election_timeout_max: 10_000,
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let builder = RaftRouter::builder(config)?;
    let mut router = builder.pipelined_append().build();
    tracing::info!("--- initialize three voters with pipelined replication");
    let log_index = router.new_cluster([0, 1, 2], []).await?;
    let target_index = log_index + 1;
    let n0 = router.get_raft_handle(&0)?;

    tracing::info!(
        target_index,
        "--- hold both data responses and every later commit-only response"
    );
    let (data_1_tx, data_1_rx) = TypeConfig::oneshot();
    let (release_1_tx, release_1_rx) = TypeConfig::oneshot();
    let (data_2_tx, data_2_rx) = TypeConfig::oneshot();
    let (release_2_tx, release_2_rx) = TypeConfig::oneshot();
    let (commit_tx, commit_rx) = TypeConfig::oneshot();
    {
        let gates = BTreeMap::from([(1, (data_1_tx, release_1_rx)), (2, (data_2_tx, release_2_rx))]);
        let gates = Mutex::new(gates);
        let commit_tx = Mutex::new(Some(commit_tx));
        router
            .set_rpc_post_hook(RpcKind::AppendEntries, move |call| {
                let RpcRequest::AppendEntries(request) = call.request else {
                    unreachable!("AppendEntries hook received another request kind");
                };
                let last_index = if let Some(entry) = request.entries.last() {
                    let log_id = entry.log_id();
                    Some(log_id.index)
                } else {
                    None
                };
                let gate = if last_index == Some(target_index) {
                    gates.lock().unwrap().remove(&call.to)
                } else {
                    None
                };
                let empty = request.entries.is_empty();
                let at_boundary = request.prev_log_id.is_some_and(|id| id.index == target_index);
                let commit_only = empty && at_boundary;
                let committed_on_2 = if commit_only && call.to == 2 {
                    commit_tx.lock().unwrap().take()
                } else {
                    None
                };
                async move {
                    if let Some((seen, release)) = gate {
                        seen.send(()).expect("test waits for each data response");
                        release.await.expect("test releases each data response");
                    }
                    if let Some(tx) = committed_on_2 {
                        tx.send(()).expect("test waits for the commit-only request");
                    }
                    if commit_only {
                        futures::future::pending::<()>().await;
                    }
                    Ok(())
                }
            })
            .await;
    }

    tracing::info!(
        target_index,
        "--- send data before the read and wait for both followers to accept it"
    );
    let write = {
        let write_router = router.clone();
        let write = TypeConfig::spawn(async move { write_router.client_request(0, "pipeline", 1).await });
        TypeConfig::timeout(WAIT_TIMEOUT, data_1_rx).await??;
        TypeConfig::timeout(WAIT_TIMEOUT, data_2_rx).await??;
        write
    };

    tracing::info!(
        target_index,
        "--- queue ReadIndex before allowing the data write to commit"
    );
    let option = LinearizerOption::new(Some(Duration::ZERO), false);
    let option = option.with_wait_timeout(Duration::from_secs(3));
    let mut read = Box::pin(n0.get_read_linearizer(option));
    {
        let status = futures::poll!(read.as_mut());
        let pending = status.is_pending();
        assert!(pending, "no follower has acknowledged an RPC sent after this read");
        // This FIFO API barrier ensures RaftCore processed the read before either release.
        n0.with_raft_state(|_| ()).await?;
        release_1_tx.send(()).expect("voter 1 response is held");
        let response = TypeConfig::timeout(WAIT_TIMEOUT, write).await???;
        let actual_index = response.log_id.index;
        assert_eq!(actual_index, target_index);
    }

    tracing::info!(
        target_index,
        "--- queue a newer commit-only request before releasing voter 2's old response"
    );
    {
        TypeConfig::timeout(WAIT_TIMEOUT, commit_rx).await??;
        let status = futures::poll!(read.as_mut());
        let pending = status.is_pending();
        assert!(
            pending,
            "the read must still be waiting when the old response is released"
        );
        release_2_tx.send(()).expect("voter 2 response is held");
        router
            .wait(&0)
            .metrics(
                |metrics| {
                    let replication = metrics.replication.as_ref();
                    let Some(replication) = replication else {
                        return false;
                    };
                    let Some(Some(matched)) = replication.get(&2) else {
                        return false;
                    };
                    matched.index == target_index
                },
                "old data response processed",
            )
            .await?;
    }

    tracing::info!(
        target_index,
        "--- the read times out because every response belongs to an older RPC"
    );
    {
        let result = TypeConfig::timeout(WAIT_TIMEOUT, read).await?;
        let Err(RaftError::APIError(LinearizableReadError::QuorumNotEnough(error))) = result else {
            panic!("expected QuorumNotEnough, got {result:?}");
        };
        let expected = btreeset! {0};
        assert_eq!(error.got, expected);
        router.clear_rpc_post_hook(RpcKind::AppendEntries).await;
        for id in [0, 1, 2] {
            router.stop_node(id).await?;
        }
    }
    Ok(())
}
