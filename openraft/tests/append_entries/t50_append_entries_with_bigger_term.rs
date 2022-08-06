use std::sync::Arc;

use anyhow::Result;
use maplit::btreeset;
use openraft::raft::AppendEntriesRequest;
use openraft::Config;
use openraft::LeaderId;
use openraft::LogId;
use openraft::RaftNetwork;
use openraft::RaftNetworkFactory;
use openraft::Vote;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// append-entries should update hard state when adding new logs with bigger term
///
/// - bring up a learner and send to it append_entries request. Check the hard state updated.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn append_entries_with_bigger_term() -> Result<()> {
    // Setup test dependencies.
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());
    let log_index = router.new_nodes_from_single(btreeset! {0}, btreeset! {1}).await?;

    // before append entries, check hard state in term 1 and vote for node 0
    router
        .assert_storage_state(1, log_index, Some(0), LogId::new(LeaderId::new(1, 0), log_index), None)
        .await?;

    // append entries with term 2 and leader_id, this MUST cause hard state changed in node 0
    let req = AppendEntriesRequest::<memstore::Config> {
        vote: Vote::new_committed(2, 1),
        prev_log_id: Some(LogId::new(LeaderId::new(1, 0), log_index)),
        entries: vec![],
        leader_commit: Some(LogId::new(LeaderId::new(1, 0), log_index)),
    };

    let resp = router.connect(0, &()).await?.send_append_entries(req).await?;
    assert!(resp.is_success());

    // after append entries, check hard state in term 2 and vote for node 1
    let mut store = router.get_storage_handle(&0)?;

    router
        .assert_storage_state_with_sto(
            &mut store,
            &0,
            2,
            log_index,
            Some(1),
            LogId::new(LeaderId::new(1, 0), log_index),
            &None,
        )
        .await?;

    Ok(())
}
