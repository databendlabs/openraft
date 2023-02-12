use std::sync::Arc;

use anyhow::Result;
use memstore::ClientRequest;
use openraft::raft::AppendEntriesRequest;
use openraft::Config;
use openraft::Entry;
use openraft::EntryPayload;
use openraft::LeaderId;
use openraft::LogId;
use openraft::RaftNetwork;
use openraft::RaftNetworkFactory;
use openraft::Vote;

use crate::fixtures::blank;
use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Cluster conflict_with_empty_entries test.
///
/// `append_entries` should get a response with non-none ConflictOpt even if the entries in message
/// is empty.
/// Otherwise if no conflict is found the leader will never be able to sync logs to a new added
/// Learner, until a next log is proposed on leader.
///
/// What does this test do?
///
/// - brings a 1 Learner node online.
///
/// - send `append_logs` message to it with empty `entries` and some non-zero `prev_log_index`.
/// - asserts that a response with ConflictOpt set.
///
/// - feed several logs to it.
///
/// - send `append_logs` message with conflicting prev_log_index and empty `entries`.
/// - asserts that a response with ConflictOpt set.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn conflict_with_empty_entries() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    router.new_raft_node(0).await;

    // Expect conflict even if the message contains no entries.

    let rpc = AppendEntriesRequest::<memstore::Config> {
        vote: Vote::new_committed(1, 1),
        prev_log_id: Some(LogId::new(LeaderId::new(1, 0), 5)),
        entries: vec![],
        leader_commit: Some(LogId::new(LeaderId::new(1, 0), 5)),
    };

    let resp = router.new_client(0, &()).await.send_append_entries(rpc).await?;
    assert!(!resp.is_success());
    assert!(resp.is_conflict());

    // Feed logs

    let rpc = AppendEntriesRequest::<memstore::Config> {
        vote: Vote::new_committed(1, 1),
        prev_log_id: None,
        entries: vec![blank(0, 0), blank(1, 1), Entry {
            log_id: LogId::new(LeaderId::new(1, 0), 2),
            payload: EntryPayload::Normal(ClientRequest {
                client: "foo".to_string(),
                serial: 1,
                status: "bar".to_string(),
            }),
        }],
        leader_commit: Some(LogId::new(LeaderId::new(1, 0), 5)),
    };

    let resp = router.new_client(0, &()).await.send_append_entries(rpc).await?;
    assert!(resp.is_success());
    assert!(!resp.is_conflict());

    // Expect a conflict with prev_log_index == 3

    let rpc = AppendEntriesRequest::<memstore::Config> {
        vote: Vote::new_committed(1, 1),
        prev_log_id: Some(LogId::new(LeaderId::new(1, 0), 3)),
        entries: vec![],
        leader_commit: Some(LogId::new(LeaderId::new(1, 0), 5)),
    };

    let resp = router.new_client(0, &()).await.send_append_entries(rpc).await?;
    assert!(!resp.is_success());
    assert!(resp.is_conflict());

    Ok(())
}
