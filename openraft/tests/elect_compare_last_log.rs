use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use fixtures::RaftRouter;
use maplit::btreeset;
use openraft::raft::Entry;
use openraft::raft::EntryPayload;
use openraft::storage::HardState;
use openraft::Config;
use openraft::LogId;
use openraft::Membership;
use openraft::RaftStorage;
use openraft::State;

#[macro_use]
mod fixtures;

/// The last_log in a vote request must be greater or equal than the local one.
///
/// - Fake a cluster with two node: with last log {2,1} and {1,2}.
/// - Bring up the cluster and only node 0 can become leader.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn elect_compare_last_log() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let router = Arc::new(RaftRouter::new(config.clone()));

    let sto0 = router.new_store(0).await;
    let sto1 = router.new_store(1).await;

    tracing::info!("--- fake store: sto0: last log: 2,1");
    {
        sto0.save_hard_state(&HardState {
            current_term: 10,
            voted_for: None,
        })
        .await?;

        sto0.append_to_log(&[&Entry {
            log_id: LogId { term: 2, index: 1 },
            payload: EntryPayload::Membership(Membership::new_single(btreeset! {0,1})),
        }])
        .await?;
    }

    tracing::info!("--- fake store: sto1: last log: 1,2");
    {
        sto1.save_hard_state(&HardState {
            current_term: 10,
            voted_for: None,
        })
        .await?;

        sto1.append_to_log(&[
            &Entry {
                log_id: LogId { term: 1, index: 1 },
                payload: EntryPayload::Membership(Membership::new_single(btreeset! {0,1})),
            },
            &Entry {
                log_id: LogId { term: 1, index: 2 },
                payload: EntryPayload::Blank,
            },
        ])
        .await?;
    }

    tracing::info!("--- bring up cluster and elect");

    router.new_raft_node_with_sto(0, sto0.clone()).await;
    router.new_raft_node_with_sto(1, sto1.clone()).await;

    router
        .wait_for_state(&btreeset! {0}, State::Leader, timeout(), "only node 0 becomes leader")
        .await?;

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(5000))
}
