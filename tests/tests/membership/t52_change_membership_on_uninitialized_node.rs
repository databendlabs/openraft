use std::sync::Arc;

use anyhow::Result;
use maplit::btreemap;
use openraft::ChangeMembers;
use openraft::Config;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Call `Raft::change_membership()` on an uninitialized node should not panic due to empty
/// membership.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn change_membership_on_uninitialized_node() -> Result<()> {
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
    let res = n0.change_membership(ChangeMembers::AddVoters(btreemap! {0=>()}), false).await;
    tracing::info!("{:?}", res);

    let err = res.unwrap_err();
    tracing::info!("{}", err);

    assert!(err.to_string().contains("forward request to"));

    Ok(())
}
