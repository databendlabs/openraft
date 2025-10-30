use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::LogIdOptionExt;
use openraft::ServerState;
use openraft::storage::RaftStateMachine;
use tokio::sync::watch;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// The logs have to be applied in log index order.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
#[ignore]
async fn total_order_apply() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    router.new_raft_node(0).await;
    router.new_raft_node(1).await;

    tracing::info!("--- initializing single node cluster");
    {
        let n0 = router.get_raft_handle(&0)?;
        n0.initialize(btreeset! {0}).await?;

        router.wait(&0, timeout()).state(ServerState::Leader, "n0 -> leader").await?;
    }

    tracing::info!("--- add one learner");
    router.add_learner(0, 1).await?;

    let (tx, rx) = watch::channel(false);

    let (_sto1, mut sm1) = router.get_storage_handle(&1)?;

    let mut prev = None;
    let h = tokio::spawn(async move {
        loop {
            if *rx.borrow() {
                break;
            }

            let (last, _) = sm1.applied_state().await.unwrap();

            if last.index() < prev {
                panic!("out of order apply");
            }
            prev = last.index();
        }
    });

    let n = 10_000;
    router.client_request_many(0, "foo", n).await?;

    // stop the log checking task.
    tx.send(true)?;
    h.await?;

    let want = n as u64;
    router
        .wait(&1u64, timeout())
        .metrics(
            |x| x.last_applied.index() >= Some(want),
            &format!("n{}.last_applied -> {}", 1, want),
        )
        .await?;

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2000))
}
