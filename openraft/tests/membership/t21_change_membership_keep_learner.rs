use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use openraft::Config;

use crate::fixtures::RaftRouter;

/// Given a cluster of voter {0,1,2} and learners {3,4,5};
/// Changing membership to {0,3,4} should not remove replication to node-5, should only remove replication to {1,2}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn change_membership_keep_learners() -> anyhow::Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    let lag_threshold = 1;

    let config = Arc::new(
        Config {
            replication_lag_threshold: lag_threshold,
            ..Default::default()
        }
        .validate()?,
    );
    let router = Arc::new(RaftRouter::new(config.clone()));

    let mut log_index = router.new_nodes_from_single(btreeset! {0,1,2}, btreeset! {3,4,5}).await?;

    tracing::info!("--- change membership to: 0,3,4");
    router.change_membership(0, btreeset! {0,3,4}).await?;
    log_index += 2;

    tracing::info!("--- write 5 logs");
    {
        router.client_request_many(0, "foo", 5).await;
        log_index += 5;

        for id in [1, 2] {
            assert!(router
                .wait(&id, timeout())
                .await?
                .log(Some(log_index), "removed voters can not receive logs")
                .await
                .is_err());
        }

        for id in [0, 3, 4, 5] {
            router
                .wait(&id, timeout())
                .await?
                .log(Some(log_index), "other voters and learners receive all logs")
                .await?;
        }
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(500))
}
