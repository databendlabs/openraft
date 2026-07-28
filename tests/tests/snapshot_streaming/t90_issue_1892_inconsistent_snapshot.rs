//! Openraft rejects protocol-impossible input early, instead of corrupting its state:
//!
//! - Installing a snapshot whose last log id can not be owned by the leader installing it, or that
//!   contradicts locally committed logs, panics at install time.
//! - A store already corrupted this way is rejected at startup by
//!   `StorageHelper::get_initial_state` with an "inverted log order" error.
//!
//! Without these checks such a snapshot survived installation with the conflicting log tail
//! retained: `RaftState::log_ids` became non-monotonic in leader id and replication to a new
//! learner later aborted the process with `unreachable!("no data to send")`.
//!
//! See: <https://github.com/databendlabs/openraft/pull/1892>

use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use openraft::Config;
use openraft::Membership;
use openraft::Vote;
use openraft::alias::SnapshotOf;
use openraft::entry::RaftEntry;
use openraft::raft::AppendEntriesRequest;
use openraft::storage::RaftLogStorage;
use openraft_memstore::MemNodeId;
use openraft_memstore::TypeConfig;

use crate::fixtures::MemRaft;
use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// The term of the foreign snapshot: greater than any term [`NODE`] has seen.
const SNAPSHOT_TERM: u64 = 3;

const NODE: MemNodeId = 0;
const HELPER: MemNodeId = 1;

const TIMEOUT: Option<Duration> = Some(Duration::from_millis(1000));

/// A snapshot whose last log id is beyond the leadership of the vote installing it is
/// protocol-impossible input: `install_full_snapshot` panics instead of installing it.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn install_snapshot_beyond_leader_vote_panics() -> anyhow::Result<()> {
    let (_config, mut router) = single_node_cluster().await?;
    let snapshot = higher_term_snapshot(&mut router).await?;

    tracing::info!("--- restart, then install the term-3 snapshot with a term-1 vote");
    restart(&mut router, NODE).await?;
    let n0 = router.get_raft_handle(&NODE)?;

    let res = n0.install_full_snapshot(Vote::new_committed(1, HELPER), snapshot).await;

    let err = res.unwrap_err();
    assert!(
        err.to_string().contains("panicked"),
        "installing a protocol-impossible snapshot must panic the node, got: {}",
        err
    );
    Ok(())
}

/// A log store purged beyond its log tail, the state `install_full_snapshot` used to leave
/// behind, is rejected at startup.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn startup_rejects_inverted_log_store() -> anyhow::Result<()> {
    let (config, mut router) = single_node_cluster().await?;

    tracing::info!("--- corrupt the store: purge to (3,0,0), above the term-1 tail by log id order");
    let (node, mut log_store, sm) = router.remove_node(NODE).unwrap();
    node.shutdown().await?;
    log_store.purge(log_id(SNAPSHOT_TERM, NODE, 0)).await?;

    let res = MemRaft::new(NODE, config, router.clone(), log_store, sm).await;

    let err = res.unwrap_err();
    assert!(
        err.to_string().contains("inverted log order"),
        "startup must reject a log store purged beyond its tail, got: {}",
        err
    );
    Ok(())
}

/// A state machine applied at a log id greater than the log tail but at a smaller index, the
/// state a crash between installing a snapshot and purging the log used to leave behind, is
/// rejected at startup.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn startup_rejects_inverted_applied_state() -> anyhow::Result<()> {
    let (config, mut router) = single_node_cluster().await?;
    let _snapshot = higher_term_snapshot(&mut router).await?;

    let (n0, log_store, _sm0) = router.remove_node(NODE).unwrap();
    n0.shutdown().await?;
    let (helper, _helper_log, helper_sm) = router.remove_node(HELPER).unwrap();
    helper.shutdown().await?;

    tracing::info!("--- pair the term-1 log store with the state machine applied at (3,0,0)");
    let res = MemRaft::new(NODE, config, router.clone(), log_store, helper_sm).await;

    let err = res.unwrap_err();
    assert!(
        err.to_string().contains("inverted log order"),
        "startup must reject a state machine ahead of the log tail at a smaller index, got: {}",
        err
    );
    Ok(())
}

/// A single-voter cluster that commits a term-1 log tail: log ids `[(0,0,0), (1,0,1)]`.
async fn single_node_cluster() -> anyhow::Result<(Arc<Config>, RaftRouter)> {
    let config = Arc::new(
        Config {
            enable_elect: false,
            // A Leader does not install a snapshot; a restart turns the node into a Follower.
            enable_leader_restore: Some(false),
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    router.new_cluster(btreeset! {NODE}, btreeset! {}).await?;
    Ok((config, router))
}

/// Builds a snapshot at term [`SNAPSHOT_TERM`], index 0, on a node outside the cluster under
/// test, standing in for the snapshot an out-of-band restore hands to a node.
///
/// The membership keeps [`NODE`] a voter, so installing the snapshot does not evict the
/// receiving node from its own config.
async fn higher_term_snapshot(router: &mut RaftRouter) -> anyhow::Result<SnapshotOf<TypeConfig, Cursor<Vec<u8>>>> {
    router.new_raft_node(HELPER).await;
    let helper = router.get_raft_handle(&HELPER)?;

    let snapshot_log_id = log_id(SNAPSHOT_TERM, NODE, 0);
    let membership = Membership::new_with_defaults(vec![btreeset! {NODE}], []);
    helper
        .append_entries(AppendEntriesRequest {
            vote: Vote::new_committed(SNAPSHOT_TERM, NODE),
            prev_log_id: None,
            entries: vec![RaftEntry::new_membership(snapshot_log_id, membership)],
            leader_commit: Some(snapshot_log_id),
        })
        .await?;
    router.wait(&HELPER, TIMEOUT).applied_index(Some(0), "helper applied").await?;

    helper.trigger().snapshot().await?;
    router.wait(&HELPER, TIMEOUT).snapshot(snapshot_log_id, "snapshot built").await?;

    Ok(helper.get_snapshot().await?.unwrap())
}

async fn restart(router: &mut RaftRouter, id: MemNodeId) -> anyhow::Result<()> {
    let (node, log_store, sm) = router.remove_node(id).unwrap();
    node.shutdown().await?;
    router.new_raft_node_with_sto(id, log_store, sm).await;
    Ok(())
}
