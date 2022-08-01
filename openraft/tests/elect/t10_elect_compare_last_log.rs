use std::option::Option::None;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::Entry;
use openraft::EntryPayload;
use openraft::LeaderId;
use openraft::LogId;
use openraft::Membership;
use openraft::RaftStorage;
use openraft::ServerState;
use openraft::Vote;

use crate::fixtures::blank;
use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// The last_log in a vote request must be greater or equal than the local one.
///
/// - Fake a cluster with two node: with last log {2,1} and {1,2}.
/// - Bring up the cluster and only node 0 can become leader.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn elect_compare_last_log() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    let mut sto0 = router.new_store();
    let mut sto1 = router.new_store();

    tracing::info!("--- fake store: sto0: last log: 2,1");
    {
        sto0.save_vote(&Vote {
            term: 10,
            node_id: 0,
            committed: false,
        })
        .await?;

        sto0.append_to_log(&[
            //
            &blank(0, 0),
            &Entry {
                log_id: LogId::new(LeaderId::new(2, 0), 1),
                payload: EntryPayload::Membership(Membership::new(vec![btreeset! {0,1}], None)),
            },
        ])
        .await?;
    }

    tracing::info!("--- fake store: sto1: last log: 1,2");
    {
        sto1.save_vote(&Vote {
            term: 10,
            node_id: 0,
            committed: false,
        })
        .await?;

        sto1.append_to_log(&[
            &blank(0, 0),
            &Entry {
                log_id: LogId::new(LeaderId::new(1, 0), 1),
                payload: EntryPayload::Membership(Membership::new(vec![btreeset! {0,1}], None)),
            },
            &blank(1, 2),
        ])
        .await?;
    }

    tracing::info!("--- bring up cluster and elect");

    router.new_raft_node_with_sto(0, sto0.clone());
    router.new_raft_node_with_sto(1, sto1.clone());

    router.wait(&0, timeout()).state(ServerState::Leader, "only node 0 becomes leader").await?;

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2000))
}
