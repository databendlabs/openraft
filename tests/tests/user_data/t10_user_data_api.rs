//! Test `Raft::user_data()` API with custom UserData type.

use std::sync::Arc;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// Test that user_data() returns a reference to the custom UserData type
/// and interior mutability works correctly.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn test_user_data_with_counter() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing single node cluster");
    router.new_cluster(btreeset! {0}, btreeset! {}).await?;

    let raft = router.get_raft_handle(&0)?;

    // Access user_data and verify it's initialized to 0
    let counter = raft.user_data();
    assert_eq!(counter.get(), 0, "counter should start at 0");

    // Test interior mutability
    counter.inc();
    counter.inc();
    counter.inc();
    assert_eq!(counter.get(), 3, "counter should be 3 after 3 increments");

    // Verify same instance is returned
    let counter2 = raft.user_data();
    assert_eq!(counter2.get(), 3, "should see the same counter state");
    assert!(std::ptr::eq(counter, counter2), "should return same instance");

    // More increments through second reference
    counter2.inc();
    assert_eq!(counter.get(), 4, "both references should see the increment");

    Ok(())
}
