use std::sync::Arc;

use anyhow::Result;
use maplit::btreeset;
use openraft::error::Fatal;
use openraft::testing::log_id;
use openraft::Config;

use crate::fixtures::ut_harness;
use crate::fixtures::MemStateMachine;
use crate::fixtures::RaftRouter;

/// Access [`RaftStateMachine`] via
/// [`Raft::with_state_machine()`](openraft::Raft::with_state_machine)
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn with_state_machine() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let n0 = router.get_raft_handle(&0)?;

    tracing::info!("--- get last applied from SM");
    {
        let applied = n0
            .with_state_machine(|sm: &mut MemStateMachine| {
                Box::pin(async move {
                    let d = sm.get_state_machine().await;
                    d.last_applied_log
                })
            })
            .await?;
        assert_eq!(applied, Some(log_id(1, 0, log_index)));
    }

    tracing::info!("--- shutting down node 0");
    n0.shutdown().await?;

    let res = n0.with_state_machine(|_sm: &mut MemStateMachine| Box::pin(async move {})).await;
    assert_eq!(Err(Fatal::Stopped), res);

    Ok(())
}
