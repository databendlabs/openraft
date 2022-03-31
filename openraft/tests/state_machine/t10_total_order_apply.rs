use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::LogIdOptionExt;
use openraft::RaftStorage;
use openraft::State;
use tokio::sync::watch;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// The logs have to be applied in log index order.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn total_order_apply() -> Result<()> {
    // Setup test dependencies.
    let config = Arc::new(Config::default().validate().expect("failed to build Raft config"));
    let mut router = RaftRouter::new(config.clone());

    router.new_raft_node(0).await;
    router.new_raft_node(1).await;

    tracing::info!("--- initializing single node cluster");

    router.initialize_with(0, btreeset![0]).await?;
    router
        .wait_for_metrics(&0u64, |x| x.state == State::Leader, timeout(), "n0.state -> Leader")
        .await?;

    tracing::info!("--- add one learner");
    router.add_learner(0, 1).await?;

    let (tx, rx) = watch::channel(false);

    let mut sto1 = router.get_storage_handle(&1)?;

    let mut prev = None;
    let h = tokio::spawn(async move {
        loop {
            if *rx.borrow() {
                break;
            }

            let (last, _) = sto1.last_applied_state().await.unwrap();

            if last.index() < prev {
                panic!("out of order apply");
            }
            prev = last.index();
        }
    });

    let n = 10_000;
    router.client_request_many(0, "foo", n).await;

    // stop the log checking task.
    tx.send(true)?;
    h.await?;

    let want = n as u64;
    router
        .wait_for_metrics(
            &1u64,
            |x| x.last_applied.index() >= Some(want),
            timeout(),
            &format!("n{}.last_applied -> {}", 1, want),
        )
        .await?;

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2000))
}
