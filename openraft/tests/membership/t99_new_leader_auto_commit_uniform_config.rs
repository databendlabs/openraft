use std::sync::Arc;

use anyhow::Result;
use maplit::btreeset;
use openraft::raft::Entry;
use openraft::raft::EntryPayload;
use openraft::raft::Membership;
use openraft::Config;
use openraft::LogId;
use openraft::Raft;
use openraft::RaftStorage;

use crate::fixtures::RaftRouter;

/// Cluster members_leader_fix_partial test.
/// TODO(xp): in discussion: whether a leader should auto commit a uniform membership config:
/// https://github.com/datafuselabs/openraft/discussions/17
///
/// - brings up 1 leader.
/// - manually append a joint config log.
/// - shutdown and restart, it should NOT add another final config log to complete the partial
/// membership changing
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn new_leader_auto_commit_uniform_config() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let router = Arc::new(RaftRouter::new(config.clone()));

    let mut n_logs = router.new_nodes_from_single(btreeset! {0}, btreeset! {}).await?;

    let sto = router.get_storage_handle(&0).await?;
    router.remove_node(0).await;

    {
        sto.append_to_log(&[&Entry {
            log_id: LogId {
                term: 1,
                index: n_logs + 1,
            },
            payload: EntryPayload::Membership(Membership::new_multi(vec![btreeset! {0}, btreeset! {0,1,2}])),
        }])
        .await?;
    }

    // A joint log and the leader should add a new final config log.
    n_logs += 2;

    let _ = n_logs;

    // To let tne router not panic
    router.new_raft_node(1).await;
    router.new_raft_node(2).await;

    let node = Raft::new(0, config.clone(), router.clone(), sto.clone());

    let _ = node;

    // node.wait(Some(Duration::from_millis(500)))
    //     .metrics(
    //         |x| x.last_log_index == want,
    //         "wait for leader to complete the final config log",
    //     )
    //     .await?;
    //
    // let final_log = sto.get_log_entries(want..=want).await?[0].clone();
    //
    // let m = match final_log.payload {
    //     EntryPayload::Membership(ref m) => m.membership.clone(),
    //     _ => {
    //         panic!("expect membership config log")
    //     }
    // };
    //
    // assert_eq!(
    //     MembershipConfig {
    //         members: btreeset! {0,1,2},
    //         members_after_consensus: None,
    //     },
    //     m
    // );

    Ok(())
}
