use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::AsyncRuntime;
use openraft::Config;
use openraft::RaftTypeConfig;
use openraft::ServerState;
use openraft_memstore::TypeConfig;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// The leader leader should expire and re-elect,
/// even when the leader is the last live node in a cluster.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn last_leader_expire() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- bring up cluster of 1 node");
    let log_index = router.new_cluster(btreeset! {0,1}, btreeset! {}).await?;

    tracing::info!(log_index, "--- shutdown node-1");
    let (n1, _sto, _sm) = router.remove_node(1).unwrap();
    n1.shutdown().await?;

    tracing::info!(log_index, "--- ");
    {
        let n0 = router.get_raft_handle(&0)?;
        let last_modify = n0.with_raft_state(|st| st.vote_last_modified()).await?;
        let last_modify = last_modify.unwrap();
        let now = <<TypeConfig as RaftTypeConfig>::AsyncRuntime as AsyncRuntime>::Instant::now();
        println!(
            "last_modify: {:?}, now: {:?}, {:?}",
            last_modify,
            now,
            now - last_modify
        );
    }

    tracing::info!(log_index, "--- node-0 should expire leader lease and step down");
    {
        router.wait(&0, timeout()).metrics(|m| m.current_leader.is_none(), "node-0 step down").await?;
        router.wait(&0, timeout()).metrics(|m| m.state != ServerState::Leader, "node-0 step down").await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2_000))
}
