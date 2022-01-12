use std::sync::Arc;

use anyhow::Result;
use maplit::btreeset;
use openraft::raft::AppendEntriesRequest;
use openraft::Config;
use openraft::LogId;
use openraft::RaftNetwork;
use openraft::State;

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
  router.new_raft_node(0).await;
  router.new_raft_node(1).await;
  router.new_raft_node(2).await;

  let mut n_logs = 0;

  // Assert all nodes are in learner state & have no entries.
  router.wait_for_log(&btreeset![0, 1, 2], n_logs, None, "empty").await?;
  router.wait_for_state(&btreeset![0, 1, 2], State::Learner, None, "empty").await?;
  router.assert_pristine_cluster().await;

  // Initialize the cluster, then assert that a stable cluster was formed & held.
  tracing::info!("--- initializing cluster");
  router.initialize_from_single_node(0).await?;
  n_logs += 1;

  router.wait_for_log(&btreeset![0, 1, 2], n_logs, None, "init").await?;
  router.assert_stable_cluster(Some(1), Some(n_logs)).await;

  // before append entries, check hard state in term 1
  router.assert_storage_state(1, n_logs, Some(0), LogId { term: 1, index: n_logs }, None).await?;

  tracing::info!("append-entries with bigger term");

  let req = AppendEntriesRequest::<memstore::ClientRequest> {
    term: 2,
    leader_id: 1,
    prev_log_id: LogId::new(1, n_logs),
    entries: vec![],
    leader_commit: LogId::new(1, n_logs),
  };

  let resp = router.send_append_entries(0, req).await?;
  assert!(resp.success());

  // before append entries, check hard state in term 2
  router.assert_storage_state_in_node(0,2, n_logs, Some(1), LogId { term: 1, index: n_logs }, None).await?;

  Ok(())
}