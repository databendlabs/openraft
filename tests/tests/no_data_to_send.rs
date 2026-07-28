//! Reproduce `unreachable!("no data to send")` in `ReplicationHandler::send_to_target`.
//!
//! A log id orders by leader id before index, so a snapshot whose `last_log_id` is at a
//! higher term than the local log tail sorts above that tail.
//!
//! 1. A single-voter group commits a term-1 log tail.
//! 2. `install_full_snapshot` installs a snapshot at term 3 and a lower index, carrying a term-1
//!    vote. Openraft does not check the snapshot's `last_log_id` against that vote, and the
//!    snapshot's log id is greater than the local one, so the conflicting tail is not truncated,
//!    only purged up to the snapshot's index.
//! 3. On restart, `StorageHelper::get_initial_state` sees `last_log_id < last_applied` and takes
//!    its "clean the hole" branch: `last_log_id` becomes the term-3 purge point while the term-1
//!    entries above it stay in the log store.
//! 4. The node elects itself at term 2 and appends above the purge point, so `RaftState::log_ids`
//!    is no longer monotonic in leader id.
//! 5. Replicating to a new learner starts at `prev` = the term-3 purge point and `last` = a term-2
//!    log id above it. `prev > last`, so `Inflight::logs` yields `Inflight::None` and
//!    `send_to_target` hits `unreachable!`.
//!
//! Run with debug assertions off:
//!
//! ```shell
//! RUSTFLAGS="-C debug-assertions=off" cargo test -p tests --test no_data_to_send
//! ```
//!
//! With debug assertions on, the `validit` check `local_committed() <= last_log_id()` in
//! `RaftState` fires at step 2 on the same inconsistency.

#![cfg_attr(feature = "bt", feature(error_generic_member_access))]

#[macro_use]
#[path = "fixtures/mod.rs"]
mod fixtures;

use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use openraft::Config;
use openraft::Membership;
use openraft::ServerState;
use openraft::Vote;
use openraft::alias::SnapshotOf;
use openraft::entry::RaftEntry;
use openraft::raft::AppendEntriesRequest;
use openraft_memstore::MemNodeId;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// The term of the installed snapshot: above the term-1 log tail, and above term 2, the term
/// the node reaches when it elects itself after the install.
const SNAPSHOT_TERM: u64 = 3;

const NODE: MemNodeId = 0;
const HELPER: MemNodeId = 1;
const LEARNER: MemNodeId = 2;

const TIMEOUT: Option<Duration> = Some(Duration::from_millis(1000));

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn add_learner_after_installing_higher_term_snapshot() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            enable_elect: false,
            // A leader does not install a snapshot, and the election has to follow the install.
            enable_leader_restore: Some(false),
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config);
    router.new_cluster(btreeset! {NODE}, btreeset! {}).await?;

    let snapshot = higher_term_snapshot(&mut router).await?;

    tracing::info!("--- restart, then install the higher term snapshot");
    restart(&mut router, NODE).await?;
    let n0 = router.get_raft_handle(&NODE)?;
    n0.install_full_snapshot(Vote::new_committed(1, HELPER), snapshot).await?;

    tracing::info!("--- restart again, so `get_initial_state` cleans the hole, then elect");
    restart(&mut router, NODE).await?;
    let n0 = router.get_raft_handle(&NODE)?;
    n0.trigger().elect(false).await?;
    router.wait(&NODE, TIMEOUT).state(ServerState::Leader, "leader at term 2").await?;

    let log_ids = n0.with_raft_state(|st| st.log_ids.clone()).await?;
    tracing::info!(?log_ids, "--- log ids are not monotonic in leader id");
    n0.add_learner(LEARNER, (), false).await?;
    Ok(())
}

/// Builds a snapshot at term [`SNAPSHOT_TERM`], index 0, on a node outside the cluster under
/// test, standing in for the snapshot an out-of-band restore hands to a node.
///
/// Index 0 is the lowest index that still leaves a term-1 entry above the purge point the
/// install creates. The membership has to keep [`NODE`] a voter, or installing the snapshot
/// would remove the receiving node from its own config.
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
