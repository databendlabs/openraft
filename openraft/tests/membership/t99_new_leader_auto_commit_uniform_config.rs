use std::sync::Arc;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::Entry;
use openraft::EntryPayload;
use openraft::LeaderId;
use openraft::LogId;
use openraft::Membership;
use openraft::Raft;
use openraft::RaftStorage;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Cluster members_leader_fix_partial test.
/// TODO(xp): in discussion: whether a leader should auto commit a uniform membership config:
/// https://github.com/datafuselabs/openraft/discussions/17
///
/// - brings up 1 leader.
/// - manually append a joint config log.
/// - shutdown and restart, it should NOT add another final config log to complete the partial
/// membership changing
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn new_leader_auto_commit_uniform_config() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_nodes_from_single(btreeset! {0}, btreeset! {}).await?;

    let mut sto = router.get_storage_handle(&0)?;
    router.remove_node(0);

    {
        sto.append_to_log(&[&Entry {
            log_id: LogId::new(LeaderId::new(1, 0), log_index + 1),
            payload: EntryPayload::Membership(Membership::new(
                vec![btreeset! {0}, btreeset! {0,1,2}],
                Some(btreeset! {}),
            )),
        }])
        .await?;
    }

    // A joint log and the leader should add a new final config log.
    log_index += 2;

    let _ = log_index;

    // To let tne router not panic
    router.new_raft_node(1);
    router.new_raft_node(2);

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
