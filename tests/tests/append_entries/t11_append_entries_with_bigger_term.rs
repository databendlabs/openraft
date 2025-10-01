use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::Vote;
use openraft::network::RPCOption;
use openraft::network::RaftNetworkFactory;
use openraft::network::v2::RaftNetworkV2;
use openraft::raft::AppendEntriesRequest;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// append-entries should update the vote when adding new logs with greater vote.
///
/// - Bring up a learner and send to it append_entries request.
///
/// Check the vote updated.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn append_entries_with_bigger_term() -> Result<()> {
    // Setup test dependencies.
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            enable_elect: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());
    let log_index = router.new_cluster(btreeset! {0}, btreeset! {1}).await?;

    // before append entries, check hard state in term 1 and vote for node 0
    router.assert_storage_state(1, log_index, Some(0), log_id(1, 0, log_index), None).await?;

    // append entries with term 2 and leader_id, this MUST cause hard state changed in node 0
    let req = AppendEntriesRequest::<openraft_memstore::TypeConfig> {
        vote: Vote::new_committed(2, 1),
        prev_log_id: Some(log_id(1, 0, log_index)),
        entries: vec![],
        leader_commit: Some(log_id(1, 0, log_index)),
    };

    let option = RPCOption::new(Duration::from_millis(1_000));

    let resp = router.new_client(0, &()).await.append_entries(req, option).await?;
    assert!(resp.is_success());

    // after append entries, check hard state in term 2 and vote for node 1
    let (mut store, mut sm) = router.get_storage_handle(&0)?;

    router
        .assert_storage_state_with_sto(
            &mut store,
            &mut sm,
            &0,
            2,
            log_index,
            Some(1),
            log_id(1, 0, log_index),
            &None,
        )
        .await?;

    Ok(())
}
