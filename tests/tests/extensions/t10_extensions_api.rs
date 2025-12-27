//! Test `Raft::extensions()` API for storing user-defined data.

use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// Counter for testing extensions with interior mutability.
#[derive(Clone, Default)]
struct Counter(Arc<AtomicU64>);

impl Counter {
    fn inc(&self) -> u64 {
        self.0.fetch_add(1, Ordering::SeqCst)
    }

    fn get(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// Test that extensions() allows storing and retrieving custom types
/// with interior mutability.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn test_extensions_with_counter() -> Result<()> {
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

    // Insert counter into extensions
    raft.extensions().insert(Counter::default());

    // Access counter and verify it's initialized to 0
    let counter = raft.extensions().get::<Counter>().expect("counter should exist");
    assert_eq!(counter.get(), 0, "counter should start at 0");

    // Test interior mutability
    counter.inc();
    counter.inc();
    counter.inc();
    assert_eq!(counter.get(), 3, "counter should be 3 after 3 increments");

    // Get another clone - shares the same Arc
    let counter2 = raft.extensions().get::<Counter>().expect("counter should still exist");
    assert_eq!(counter2.get(), 3, "should see the same counter state");

    // More increments
    counter2.inc();
    assert_eq!(counter2.get(), 4, "should see the increment");

    // Test that non-existent types return None
    #[derive(Clone)]
    struct NonExistent;
    assert!(raft.extensions().get::<NonExistent>().is_none());

    // Test contains
    assert!(raft.extensions().contains::<Counter>());
    assert!(!raft.extensions().contains::<NonExistent>());

    Ok(())
}
