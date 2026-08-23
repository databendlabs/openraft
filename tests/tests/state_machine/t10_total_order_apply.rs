use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::LogIdOptionExt;
use openraft::async_runtime::WatchReceiver;
use openraft::async_runtime::WatchSender;
use openraft::storage::RaftStateMachine;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

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

    tracing::info!("--- bring up one leader and one learner");
    router.new_cluster(btreeset! {0}, btreeset! {1}).await?;

    let (tx, rx) = TypeConfig::watch_channel(false);

    let (_sto1, mut sm1) = router.get_storage_handle(&1)?;

    let mut prev = None;
    let h = TypeConfig::spawn(async move {
        loop {
            if *rx.borrow_watched() {
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
