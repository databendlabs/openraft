use std::sync::Arc;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Get config via [`Raft::config`](openraft::Raft::config)
#[async_entry::test(worker_threads = 4, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn raft_config() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_tick: false,
            election_timeout_min: 123,
            election_timeout_max: 124,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;

    tracing::info!(log_index, "--- get config");
    {
        let n0 = router.get_raft_handle(&0)?;
        let c = n0.config();

        #[allow(clippy::bool_assert_comparison)]
        {
            assert_eq!(c.enable_tick, false);
        }
        assert_eq!(c.election_timeout_min, 123);
        assert_eq!(c.election_timeout_max, 124);
    }

    Ok(())
}
