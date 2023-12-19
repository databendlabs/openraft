use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::error::ClientWriteError;
use openraft::error::ForwardToLeader;
use openraft::error::RaftError;
use openraft::raft::AppendEntriesRequest;
use openraft::testing::log_id;
use openraft::Config;
use openraft::Vote;
use openraft_memstore::ClientRequest;
use openraft_memstore::IntoMemClientRequest;
use tokio::sync::oneshot;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Client write will receive a [`ForwardToLeader`] error because of log reversion, when leader
/// quit, even after log is appended.
///
/// [`ForwardToLeader`]: openraft::error::ForwardToLeader
#[async_entry::test(worker_threads = 4, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn write_when_leader_quit_and_log_revert() -> Result<()> {
    let config = Arc::new(
        Config {
            heartbeat_interval: 100,
            election_timeout_min: 200,
            election_timeout_max: 300,
            enable_tick: false,
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let log_index = router.new_cluster(btreeset! {0,1}, btreeset! {}).await?;

    tracing::info!(log_index, "--- block replication so that no log will be committed");
    router.set_unreachable(1, true);

    let (tx, rx) = oneshot::channel();

    tracing::info!(log_index, "--- write a log in another task");
    {
        let n0 = router.get_raft_handle(&0)?;
        tokio::spawn(async move {
            let res = n0.client_write(ClientRequest::make_request("cli", 1)).await;
            tx.send(res).unwrap();
        });
    }

    // wait for log to be appended on leader, and response channel is installed.
    tokio::time::sleep(Duration::from_millis(500)).await;

    tracing::info!(log_index, "--- force node 0 to give up leadership");
    {
        let n0 = router.get_raft_handle(&0)?;
        let append_res = n0
            .append_entries(AppendEntriesRequest {
                // From node 2, with a higher term 10
                vote: Vote::new_committed(10, 1),
                // log_index+1 is the log index the client tries to write, in previous step.
                // This log conflict with the log the client written, will cause raft to revert log.
                prev_log_id: Some(log_id(10, 1, log_index + 1)),

                entries: vec![],
                leader_commit: None,
            })
            .await?;

        tracing::info!(log_index, "--- append_res: {:?}", append_res);
    }

    let write_res = rx.await?;
    tracing::info!(log_index, "--- write_res: {:?}", write_res);

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

/// Client write will still receive an OK response, as soon as log is committed, even when leader is
/// switched.
///
/// [`ForwardToLeader`]: openraft::error::ForwardToLeader
#[async_entry::test(worker_threads = 4, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn write_when_leader_switched() -> Result<()> {
    let config = Arc::new(
        Config {
            heartbeat_interval: 100,
            election_timeout_min: 200,
            election_timeout_max: 300,
            enable_tick: false,
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let log_index = router.new_cluster(btreeset! {0,1}, btreeset! {}).await?;

    tracing::info!(log_index, "--- block replication so that no log will be committed");
    router.set_unreachable(1, true);

    let (tx, rx) = oneshot::channel();

    tracing::info!(log_index, "--- write a log in another task");
    {
        let n0 = router.get_raft_handle(&0)?;
        tokio::spawn(async move {
            let res = n0.client_write(ClientRequest::make_request("cli", 1)).await;
            tx.send(res).unwrap();
        });
    }

    // wait for log to be appended on leader, and response channel is installed.
    tokio::time::sleep(Duration::from_millis(500)).await;

    tracing::info!(log_index, "--- force node 0 to give up leadership, inform it to commit");
    {
        let n0 = router.get_raft_handle(&0)?;
        let append_res = n0
            .append_entries(AppendEntriesRequest {
                // From node 2, with a higher term 10
                vote: Vote::new_committed(10, 1),
                // log_index+1 is the log index the client tries to write, in previous step.
                // This matches the log on node-0.
                prev_log_id: Some(log_id(1, 0, log_index + 1)),

                entries: vec![],

                // Inform node-0 to commit the pending log.
                leader_commit: Some(log_id(1, 0, log_index + 1)),
            })
            .await?;

        dbg!(&append_res);
        tracing::info!(log_index, "--- append_res: {:?}", append_res);
    }

    let write_res = rx.await?;
    tracing::info!(log_index, "--- write_res: {:?}", write_res);

    let ok_resp = write_res?;
    assert_eq!(ok_resp.log_id, log_id(1, 0, log_index + 1), "client write committed");

    Ok(())
}
