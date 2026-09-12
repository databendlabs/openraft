use std::sync::Arc;

use anyhow::Result;
use openraft::Config;
use openraft::errors::ClientWriteError;
use openraft::errors::ForwardToLeader;
use openraft::errors::RaftError;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// A membership request to an uninitialized node should be rejected before inspecting its empty
/// membership.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn add_learner_on_uninitialized_node() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    router.new_raft_node(0).await;

    let n0 = router.get_raft_handle(&0)?;
    let err = n0.add_learner(0, (), false).await.unwrap_err();
    assert_eq!(
        RaftError::APIError(ClientWriteError::ForwardToLeader(ForwardToLeader::empty())),
        err
    );

    Ok(())
}
