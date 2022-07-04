use std::sync::Arc;

use anyhow::Result;
use maplit::btreeset;
use openraft::raft::AppendEntriesRequest;
use openraft::Config;
use openraft::LogId;
use openraft::RaftNetwork;
use openraft_memstore::ClientRequest;

use crate::fixtures::RaftRouter;

/// append-entries should update hard state when adding new logs with bigger term
///
/// - bring up a learner and send to it append_entries request. Check the hard state updated.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn append_entries_with_bigger_term() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let router = Arc::new(RaftRouter::new(config.clone()));
    let n_logs = router.new_nodes_from_single(btreeset! {0}, btreeset! {1}).await?;

    // before append entries, check hard state in term 1 and vote for node 0
    router.assert_storage_state(1, n_logs, Some(0), LogId { term: 1, index: n_logs }, None).await?;

    tracing::info!("append-entries with bigger term");
    // append entries with term 2 and leader_id, this MUST cause hard state changed in node 0
    let req = AppendEntriesRequest::<ClientRequest> {
        term: 2,
        leader_id: 1,
        prev_log_id: Some(LogId::new(1, n_logs)),
        entries: vec![],
        leader_commit: Some(LogId::new(1, n_logs)),
    };

    let resp = router.send_append_entries(0, req).await?;
    assert!(resp.success);

    // after append entries, check hard state in term 2  and vote for node 1
    router
        .assert_storage_state_in_node(0, 2, n_logs, Some(1), LogId { term: 1, index: n_logs }, None)
        .await?;

    Ok(())
}
