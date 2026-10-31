use anyhow::Result;
use openraft::Config;
use openraft::RaftLogReader;
use openraft::Vote;
use openraft::async_runtime::OneshotSender;
use openraft::async_runtime::WatchReceiver;
use openraft::errors::ClientWriteError;
use openraft::errors::ForwardReason;
use openraft::errors::ForwardToLeader;
use openraft::impls::ProgressResponder;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::ClientRequest;
use openraft_memstore::ClientResponse;
use openraft_memstore::IntoMemClientRequest;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::WAIT_TIMEOUT;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// Regression test for <https://github.com/databendlabs/openraft/issues/2247>.
///
/// Applying a retained log tail while snapshot installation is pending must discard covered
/// responders and preserve the tail responder. Otherwise, the apply panics on a covered responder
/// whose log ID has already been removed from the engine, before `PurgeLog` can drain it.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn snapshot_pending_responders_before_tail_apply() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initialize voters 0, 1, 2 with node 0 leading");
    let log_index = router.new_cluster([0, 1, 2], []).await?;
    let n0 = router.get_raft_handle(&0)?;
    let base_log_id = log_id(1, 0, log_index);
    let snapshot_log_id = log_id(1, 0, log_index + 2);
    let tail_log_id = log_id(1, 0, log_index + 3);
    let new_vote = Vote::new_committed(2, 1);

    tracing::info!(
        log_index,
        "--- isolate node 0 and retain three uncommitted client responders"
    );
    let (mut responses, entries) = {
        router.set_unreachable(0, true);
        let mut responses = vec![];
        for serial in 1..=3 {
            let (responder, committed, completed) = ProgressResponder::new();
            let request = ClientRequest::make_request("pending", serial);
            n0.client_write_ff(request, Some(responder)).await?;
            responses.push((committed, completed));
        }
        router.wait(&0).log_index(Some(tail_log_id.index), "three writes appended").await?;

        let (mut store, _) = router.get_storage_handle(&0)?;
        let entries = store.try_get_log_entries(log_index + 1..=tail_log_id.index).await?;
        (responses, entries)
    };

    tracing::info!(%snapshot_log_id, "--- snapshot the first two writes on node 2");
    let snapshot = {
        let n2 = router.get_raft_handle(&2)?;
        let response = n2
            .append_entries(AppendEntriesRequest {
                vote: new_vote,
                prev_log_id: Some(base_log_id),
                entries,
                leader_commit: Some(snapshot_log_id),
            })
            .await?;
        assert_eq!(AppendEntriesResponse::Success, response);
        router.wait(&2).applied_index(Some(snapshot_log_id.index), "snapshot prefix applied").await?;

        n2.trigger().snapshot().await?;
        router.wait(&2).snapshot(snapshot_log_id, "snapshot built").await?;
        let snapshot = n2.get_snapshot().await?;
        snapshot.expect("node 2 has a snapshot")
    };

    tracing::info!(%new_vote, %tail_log_id, "--- step down node 0 while retaining its uncommitted tail");
    {
        let response = n0
            .append_entries(AppendEntriesRequest {
                vote: new_vote,
                prev_log_id: Some(tail_log_id),
                entries: vec![],
                leader_commit: Some(base_log_id),
            })
            .await?;
        assert_eq!(AppendEntriesResponse::Success, response);
    }

    tracing::info!(
        log_index,
        "--- pause the state-machine worker so snapshot installation cannot finish"
    );
    let release_worker = {
        let (started_tx, started_rx) = TypeConfig::oneshot();
        let (release_tx, release_rx) = TypeConfig::oneshot::<()>();
        n0.external_state_machine_request(move |_sm| {
            Box::pin(async move {
                started_tx.send(()).expect("test waits for the worker to pause");
                let _release = release_rx.await;
            })
        })
        .await?;
        TypeConfig::timeout(WAIT_TIMEOUT, started_rx).await??;
        release_tx
    };

    tracing::info!(%snapshot_log_id, "--- submit the snapshot while its completion and purge remain blocked");
    let install = {
        let installing_node = n0.clone();
        let install = TypeConfig::spawn(async move { installing_node.install_full_snapshot(new_vote, snapshot).await });
        router.wait(&0).committed_index(Some(snapshot_log_id.index), "snapshot accepted").await?;
        let metrics = n0.metrics();
        let applied = metrics.borrow_watched().last_applied;
        let expected_applied = Some(base_log_id);
        assert_eq!(expected_applied, applied);
        install
    };

    tracing::info!(%tail_log_id, "--- commit the retained tail before purge can discard covered responders");
    {
        let committing_node = n0.clone();
        let append = TypeConfig::spawn(async move {
            committing_node
                .append_entries(AppendEntriesRequest {
                    vote: new_vote,
                    prev_log_id: Some(tail_log_id),
                    entries: vec![],
                    leader_commit: Some(tail_log_id),
                })
                .await
        });

        let (committed, completed) = responses.pop().expect("the tail has a responder");
        let committed = TypeConfig::timeout(WAIT_TIMEOUT, committed).await??;
        assert_eq!(tail_log_id, committed);

        release_worker.send(()).expect("the state-machine worker is paused");
        let installed = TypeConfig::timeout(WAIT_TIMEOUT, install).await???;
        assert_eq!(new_vote, installed.vote);
        let response = TypeConfig::timeout(WAIT_TIMEOUT, append).await???;
        assert_eq!(AppendEntriesResponse::Success, response);
        router.wait(&0).applied_index(Some(tail_log_id.index), "tail applied").await?;

        let result = TypeConfig::timeout(WAIT_TIMEOUT, completed).await??;
        let response = result?;
        assert_eq!(tail_log_id, response.log_id);
        let expected_data = ClientResponse(Some("request-2".to_string()));
        assert_eq!(expected_data, response.data);
        assert_eq!(None, response.membership);
    }

    tracing::info!(%snapshot_log_id, "--- both covered responders report LogEntryDiscarded");
    {
        let expected = ClientWriteError::LogEntryDiscarded(ForwardToLeader {
            leader_id: Some(1),
            leader_node: Some(()),
            reason: ForwardReason::NotLeader,
        });
        for (_committed, completed) in responses {
            let result = TypeConfig::timeout(WAIT_TIMEOUT, completed).await??;
            let error = result.expect_err("the snapshot superseded this pending write");
            assert_eq!(expected, error);
        }
    }

    Ok(())
}
