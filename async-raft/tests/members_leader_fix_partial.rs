use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_raft::raft::Entry;
use async_raft::raft::EntryConfigChange;
use async_raft::raft::EntryPayload;
use async_raft::raft::MembershipConfig;
use async_raft::Config;
use async_raft::LogId;
use async_raft::Raft;
use async_raft::RaftStorage;
use fixtures::RaftRouter;
use maplit::btreeset;

#[macro_use]
mod fixtures;

/// Cluster members_leader_fix_partial test.
///
/// - brings up 1 leader.
/// - manually append a joint config log.
/// - shutdown and restart, it should add another final config log to complete the partial
/// membership changing
///
/// RUST_LOG=async_raft,memstore,members_leader_fix_partial=trace cargo test -p async-raft --test
/// members_leader_fix_partial
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn members_leader_fix_partial() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    // Setup test dependencies.
    let config = Arc::new(Config::build("test".into()).validate().expect("failed to build Raft config"));
    let router = Arc::new(RaftRouter::new(config.clone()));

    let mut want = router.new_nodes_from_single(btreeset! {0}, btreeset! {}).await?;

    let sto = router.get_storage_handle(&0).await?;
    router.remove_node(0).await;

    {
        sto.append_to_log(&[&Entry {
            log_id: LogId {
                term: 1,
                index: want + 1,
            },
            payload: EntryPayload::ConfigChange(EntryConfigChange {
                membership: MembershipConfig {
                    members: btreeset! {0},
                    members_after_consensus: Some(btreeset! {0,1,2}),
                },
            }),
        }])
        .await?;
    }

    // A joint log and the leader should add a new final config log.
    want += 2;

    // To let tne router not panic
    router.new_raft_node(1).await;
    router.new_raft_node(2).await;

    let node = Raft::new(0, config.clone(), router.clone(), sto.clone());

    node.wait(Some(Duration::from_millis(500)))
        .metrics(
            |x| x.last_log_index == want,
            "wait for leader to complete the final config log",
        )
        .await?;

    let final_log = sto.get_log_entries(want, want + 1).await?[0].clone();

    let m = match final_log.payload {
        EntryPayload::ConfigChange(ref m) => m.membership.clone(),
        _ => {
            panic!("expect membership config log")
        }
    };

    assert_eq!(
        MembershipConfig {
            members: btreeset! {0,1,2},
            members_after_consensus: None,
        },
        m
    );

    Ok(())
}
