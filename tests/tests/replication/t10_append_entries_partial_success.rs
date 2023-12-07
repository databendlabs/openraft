use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// RaftNetwork::send_append_entries can return a partial success.
/// For example, it tries to send log entries `[1-2..2-10]`, the application is allowed to send just
/// `[1-2..1-3]` and return `PartialSuccess(1-3)`.
#[async_entry::test(worker_threads = 4, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn append_entries_partial_success() -> Result<()> {
    let config = Arc::new(Config { ..Default::default() }.validate()?);

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster(btreeset! {0,1}, btreeset! {}).await?;

    let quota = 2;
    let n = 5;

    tracing::info!(
        log_index,
        "--- set append-entries quota to {}, wirte {} entries",
        quota,
        n
    );
    {
        router.set_append_entries_quota(Some(quota));

        let r = router.clone();
        tokio::spawn(async move {
            // client request will be blocked due to limited quota=2
            r.client_request_many(0, "0", n as usize).await.unwrap();
        });
        log_index += quota;

        router.wait(&0, timeout()).applied_index(Some(log_index), format!("{} writes", quota)).await?;

        log_index += 1;
        tracing::info!(log_index, "--- can not send log at index {}", log_index,);

        let res = router
            .wait(&0, timeout())
            .applied_index(Some(log_index), format!("log index {} is limited by quota", log_index))
            .await;

        assert!(res.is_err(), "log index {} is limited by quota", log_index);
    }

    tracing::info!(log_index, "--- extend quota by 1, send 1 log at index {}", log_index,);
    {
        router.set_append_entries_quota(Some(1));
        router
            .wait(&0, timeout())
            .applied_index(Some(log_index), format!("log index {} can be replicated", log_index))
            .await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2_000))
}
