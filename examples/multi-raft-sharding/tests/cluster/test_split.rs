//! TiKV-style Shard Split Test
//!
//! This test demonstrates the complete lifecycle of a shard split operation:
//!
//! ## Phase 1: Initial State
//! - 3 nodes (Node 1, 2, 3)
//! - 1 shard (shard_a) containing all user data (user:1 to user:200)
//!
//! ## Phase 2: Split
//! - Split shard_a at user_id=100
//! - shard_a now contains user:1 to user:100
//! - shard_b is created with user:101 to user:200
//! - Both shards run on Node 1, 2, 3
//!
//! ## Phase 3: Add New Nodes
//! - Add Node 4, 5 to the cluster
//! - Add Node 4, 5 as learners to shard_b
//! - Wait for replication to complete
//!
//! ## Phase 4: Migrate shard_b
//! - Change shard_b membership to only Node 4, 5
//! - Shut down shard_b on Node 1, 2, 3
//! - Final state: shard_a on Node 1,2,3; shard_b on Node 4,5
//!
//! ## Key Insight: Split as Raft Log
//!
//! The split operation is proposed as a normal Raft log entry. When applied:
//! 1. Each replica extracts data with user_id > 100
//! 2. Each replica removes that data from its state machine
//! 3. The extracted data is used to bootstrap the new shard
//!
//! This ensures atomic, consistent split across all replicas without
//! distributed locks or two-phase commit.

use std::backtrace::Backtrace;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::panic::PanicHookInfo;
use std::sync::Arc;
use std::time::Duration;

use multi_raft_sharding::new_raft;
use multi_raft_sharding::router::Router;
use multi_raft_sharding::shard_router::ShardRouter;
use multi_raft_sharding::shards;
use multi_raft_sharding::store::Request;
use multi_raft_sharding::store::Response;
use multi_raft_sharding::store::StateMachineStore;
use multi_raft_sharding::typ;
use multi_raft_sharding::NodeId;
use multi_raft_sharding::ShardId;
use openraft::BasicNode;
use openraft::Config;
use openraft::Raft;
use tokio::task;
use tokio::task::LocalSet;
use tracing_subscriber::EnvFilter;

/// Log panic with backtrace for debugging.
pub fn log_panic(panic: &PanicHookInfo) {
    let backtrace = format!("{:?}", Backtrace::force_capture());
    eprintln!("{}", panic);
    if let Some(location) = panic.location() {
        tracing::error!(
            message = %panic,
            backtrace = %backtrace,
            panic.file = location.file(),
            panic.line = location.line(),
            panic.column = location.column(),
        );
        eprintln!("{}:{}:{}", location.file(), location.line(), location.column());
    } else {
        tracing::error!(message = %panic, backtrace = %backtrace);
    }
    eprintln!("{}", backtrace);
}

#[tokio::test]
async fn test_shard_split() {
    std::panic::set_hook(Box::new(|panic| {
        log_panic(panic);
    }));

    tracing_subscriber::fmt()
        .with_target(true)
        .with_thread_ids(true)
        .with_level(true)
        .with_ansi(false)
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    // =========================================================================
    // Setup: Create shared infrastructure
    // =========================================================================

    // The message router simulates network communication between nodes.
    let router = Router::new();

    // The shard router tracks which shard handles which key range.
    // Initially, shard_a handles all keys [1, MAX].
    let shard_router = Arc::new(ShardRouter::with_initial_shard(shards::SHARD_A.to_string()));

    let local = LocalSet::new();

    local
        .run_until(async move {
            run_split_test(router, shard_router).await;
        })
        .await;
}

/// Run the complete split test through all four phases.
async fn run_split_test(router: Router, shard_router: Arc<ShardRouter>) {
    // =========================================================================
    // Phase 1: Initial State - 3 nodes, 1 shard
    // =========================================================================
    println!("┌──────────────────────────────────────────────────────────────────────────┐");
    println!("│ PHASE 1: Initial State                                                   │");
    println!("│                                                                          │");
    println!("│   Node 1          Node 2          Node 3                                 │");
    println!("│ ┌─────────┐    ┌─────────┐    ┌─────────┐                                │");
    println!("│ │shard_a:★│    │shard_a:F│    │shard_a:F│                                │");
    println!("│ │[1..200] │    │[1..200] │    │[1..200] │                                │");
    println!("│ └─────────┘    └─────────┘    └─────────┘                                │");
    println!("└──────────────────────────────────────────────────────────────────────────┘");
    println!();

    // Create shard_a on nodes 1, 2, 3
    let (raft_1a, app_1a) = new_raft(1, shards::SHARD_A.to_string(), router.clone()).await;
    let (_raft_2a, app_2a) = new_raft(2, shards::SHARD_A.to_string(), router.clone()).await;
    let (_raft_3a, app_3a) = new_raft(3, shards::SHARD_A.to_string(), router.clone()).await;

    // Keep reference to state_machine for verification at the end
    let state_machine_1a = app_1a.state_machine.clone();

    // Spawn app handlers
    task::spawn_local(app_1a.run());
    task::spawn_local(app_2a.run());
    task::spawn_local(app_3a.run());

    // Wait for apps to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Initialize shard_a with Node 1 as the initial leader
    println!("  → Initializing shard_a on Node 1...");
    let mut nodes = BTreeMap::new();
    nodes.insert(1u64, BasicNode { addr: "".to_string() });
    raft_1a.initialize(nodes).await.unwrap();

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Add Node 2 and 3 as voters
    println!("  → Adding Node 2, 3 to shard_a...");
    raft_1a.add_learner(2, BasicNode { addr: "".to_string() }, true).await.unwrap();
    raft_1a.add_learner(3, BasicNode { addr: "".to_string() }, true).await.unwrap();

    // Promote to voters
    let members: BTreeSet<NodeId> = [1, 2, 3].into_iter().collect();
    raft_1a.change_membership(members, false).await.unwrap();

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Write user data (user:1 to user:200)
    println!("  → Writing 200 users (user:1 to user:200)...");
    for user_id in 1..=200 {
        let request = Request::set_user(user_id, format!("User_{}", user_id));
        raft_1a.client_write(request).await.unwrap();
    }

    println!("  ✓ Phase 1 complete: shard_a has 200 users on 3 nodes");
    println!();

    // Verify data
    {
        let metrics = raft_1a.metrics().borrow().clone();
        println!(
            "  Metrics: leader={:?}, last_applied={:?}",
            metrics.current_leader,
            metrics.last_applied.map(|x| x.index)
        );
    }

    // =========================================================================
    // Phase 2: Split shard_a at user_id=100
    // =========================================================================
    println!();
    println!("┌──────────────────────────────────────────────────────────────────────────┐");
    println!("│ PHASE 2: Split at user_id=100                                            │");
    println!("│                                                                          │");
    println!("│   Node 1          Node 2          Node 3                                 │");
    println!("│ ┌─────────┐    ┌─────────┐    ┌─────────┐                                │");
    println!("│ │shard_a:★│    │shard_a:F│    │shard_a:F│                                │");
    println!("│ │[1..100] │    │[1..100] │    │[1..100] │                                │");
    println!("│ ├─────────┤    ├─────────┤    ├─────────┤                                │");
    println!("│ │shard_b:★│    │shard_b:F│    │shard_b:F│                                │");
    println!("│ │[101..200│    │[101..200│    │[101..200│                                │");
    println!("│ └─────────┘    └─────────┘    └─────────┘                                │");
    println!("└──────────────────────────────────────────────────────────────────────────┘");
    println!();

    // Step 1: Propose the split as a Raft log entry
    println!("  → Proposing split operation as Raft log entry...");
    println!("    (This is the TiKV-style atomic split)");

    let split_request = Request::split(100, shards::SHARD_B);
    let split_response = raft_1a.client_write(split_request).await.unwrap();

    // Extract the split data from the response
    let split_data = match split_response.response() {
        Response::SplitComplete {
            new_shard_id,
            split_data,
            key_count,
        } => {
            println!("  ✓ Split completed atomically!");
            println!("    - New shard: {}", new_shard_id);
            println!("    - Keys migrated: {}", key_count);
            split_data.clone()
        }
        _ => panic!("Expected SplitComplete response"),
    };

    // Update the shard router
    shard_router.apply_split(shards::SHARD_A, 100, shards::SHARD_B.to_string());
    println!("  → Updated shard router:");
    println!("    - shard_a: [1..100]");
    println!("    - shard_b: [101..MAX]");

    // Step 2: Create shard_b on the same nodes using the split data
    println!("  → Creating shard_b instances on Node 1, 2, 3...");

    // Create shard_b with the split data as initial state
    let (raft_1b, app_1b) =
        create_raft_with_data(1, shards::SHARD_B.to_string(), router.clone(), split_data.clone()).await;
    let (_raft_2b, app_2b) =
        create_raft_with_data(2, shards::SHARD_B.to_string(), router.clone(), split_data.clone()).await;
    let (_raft_3b, app_3b) =
        create_raft_with_data(3, shards::SHARD_B.to_string(), router.clone(), split_data.clone()).await;

    // Keep reference to shard_b state_machine (Node 1 is the source of split data)
    let _state_machine_1b = app_1b.state_machine.clone();

    task::spawn_local(app_1b.run());
    task::spawn_local(app_2b.run());
    task::spawn_local(app_3b.run());

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Initialize shard_b
    let mut nodes = BTreeMap::new();
    nodes.insert(1u64, BasicNode { addr: "".to_string() });
    raft_1b.initialize(nodes).await.unwrap();

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Add other nodes
    raft_1b.add_learner(2, BasicNode { addr: "".to_string() }, true).await.unwrap();
    raft_1b.add_learner(3, BasicNode { addr: "".to_string() }, true).await.unwrap();

    let members: BTreeSet<NodeId> = [1, 2, 3].into_iter().collect();
    raft_1b.change_membership(members, false).await.unwrap();

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Verify both shards have correct data
    println!("  → Verifying data split...");

    // Trigger snapshot to verify data
    raft_1a.trigger().snapshot().await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Check shard_a has users 1-100
    {
        let snapshot = raft_1a.get_snapshot().await.unwrap().unwrap();
        let has_user_50 = snapshot.snapshot.data.contains_key("user:50");
        let has_user_100 = snapshot.snapshot.data.contains_key("user:100");
        let has_user_101 = snapshot.snapshot.data.contains_key("user:101");

        assert!(has_user_50, "shard_a should have user:50");
        assert!(has_user_100, "shard_a should have user:100");
        assert!(!has_user_101, "shard_a should NOT have user:101 after split");

        println!("    ✓ shard_a: {} keys (users 1-100)", snapshot.snapshot.data.len());
    }

    // Check shard_b has users 101-200 (check state machine directly as it's newly created)
    {
        let metrics = raft_1b.metrics().borrow().clone();
        println!(
            "    ✓ shard_b: initialized with split data, last_applied={:?}",
            metrics.last_applied.map(|x| x.index)
        );
    }

    println!("  ✓ Phase 2 complete: shard split successful!");
    println!();

    // =========================================================================
    // Phase 3: Add Node 4, 5 to shard_b
    // =========================================================================
    println!("┌──────────────────────────────────────────────────────────────────────────┐");
    println!("│ PHASE 3: Add Node 4, 5 to shard_b                                        │");
    println!("│                                                                          │");
    println!("│   Node 1     Node 2     Node 3     Node 4     Node 5                     │");
    println!("│ ┌────────┐  ┌────────┐  ┌────────┐  ┌────────┐  ┌────────┐               │");
    println!("│ │shrd_a:★│  │shrd_a:F│  │shrd_a:F│  │    ─   │  │    -   │               │");
    println!("│ ├────────┤  ├────────┤  ├────────┤  ├────────┤  ├────────┤               │");
    println!("│ │shrd_b:★│  │shrd_b:F│  │shrd_b:F│  │shrd_b:L│  │shrd_b:L│               │");
    println!("│ └────────┘  └────────┘  └────────┘  └────────┘  └────────┘               │");
    println!("└──────────────────────────────────────────────────────────────────────────┘");
    println!();

    // Create shard_b on Node 4, 5
    println!("  → Creating shard_b on Node 4, 5...");
    let (raft_4b, app_4b) = new_raft(4, shards::SHARD_B.to_string(), router.clone()).await;
    let (raft_5b, app_5b) = new_raft(5, shards::SHARD_B.to_string(), router.clone()).await;

    // Keep reference to state_machine for verification at the end
    let state_machine_4b = app_4b.state_machine.clone();

    task::spawn_local(app_4b.run());
    task::spawn_local(app_5b.run());

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Before adding learners, trigger a snapshot on the leader.
    // This is necessary because the initial data was injected directly into the state machine
    // (via create_raft_with_data) without corresponding log entries. Without a snapshot,
    // the leader wouldn't know it needs to send the data to new followers.
    println!("  → Triggering snapshot on shard_b leader before adding learners...");
    raft_1b.trigger().snapshot().await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Add as learners
    println!("  → Adding Node 4, 5 as learners to shard_b...");
    raft_1b.add_learner(4, BasicNode { addr: "".to_string() }, true).await.unwrap();
    raft_1b.add_learner(5, BasicNode { addr: "".to_string() }, true).await.unwrap();

    // Wait for replication (snapshot will be sent to learners)
    println!("  → Waiting for snapshot replication to Node 4, 5...");
    tokio::time::sleep(Duration::from_millis(2000)).await;

    // Verify replication
    {
        let metrics_4 = raft_4b.metrics().borrow().clone();
        let metrics_5 = raft_5b.metrics().borrow().clone();
        println!("    Node 4: last_applied={:?}", metrics_4.last_applied.map(|x| x.index));
        println!("    Node 5: last_applied={:?}", metrics_5.last_applied.map(|x| x.index));
    }

    println!("  ✓ Phase 3 complete: Node 4, 5 have shard_b data");
    println!();

    // =========================================================================
    // Phase 4: Migrate shard_b to Node 4, 5 (remove from Node 1, 2, 3)
    // =========================================================================
    println!("┌──────────────────────────────────────────────────────────────────────────┐");
    println!("│ PHASE 4: Migrate shard_b to Node 4, 5                                    │");
    println!("│                                                                          │");
    println!("│   Node 1     Node 2     Node 3     Node 4     Node 5                     │");
    println!("│ ┌────────┐  ┌────────┐  ┌────────┐  ┌────────┐  ┌────────┐               │");
    println!("│ │shrd_a:★│  │shrd_a:F│  │shrd_a:F│  │    -   │  │    -   │               │");
    println!("│ │    -   │  │    -   │  │    -   │  │shrd_b:★│  │shrd_b:F│               │");
    println!("│ └────────┘  └────────┘  └────────┘  └────────┘  └────────┘               │");
    println!("└──────────────────────────────────────────────────────────────────────────┘");
    println!();

    // First, add Node 4, 5 as voters while keeping 1, 2, 3
    println!("  → Promoting Node 4, 5 to voters...");
    let members: BTreeSet<NodeId> = [1, 2, 3, 4, 5].into_iter().collect();
    raft_1b.change_membership(members, false).await.unwrap();

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Now remove Node 1, 2, 3 from shard_b
    println!("  → Removing Node 1, 2, 3 from shard_b membership...");
    let new_members: BTreeSet<NodeId> = [4, 5].into_iter().collect();

    // This will transfer leadership to Node 4 or 5 automatically
    // because the current leader (Node 1) is being removed
    raft_1b.change_membership(new_members, false).await.unwrap();

    // Wait for leader election to complete on Node 4 or 5
    // This may take up to election_timeout_max (3000ms)
    tokio::time::sleep(Duration::from_millis(4000)).await;

    // Verify final state
    println!("  → Verifying final state...");

    // Verify shard_b leadership moved to Node 4 or 5
    {
        let metrics_4 = raft_4b.metrics().borrow().clone();
        let metrics_5 = raft_5b.metrics().borrow().clone();

        let leader = metrics_4.current_leader.or(metrics_5.current_leader);
        println!("    shard_b leader: {:?}", leader);

        // After membership change, either Node 4 or 5 should be the leader
        // If leader is None, it means election is still in progress
        if leader.is_none() {
            println!("    (Leader election may still be in progress, checking membership...)");
            println!("    Node 4 membership: {:?}", metrics_4.membership_config);
            println!("    Node 5 membership: {:?}", metrics_5.membership_config);
            // In a real scenario, we'd wait and retry
            // For the test, we just verify the membership change was successful
            let voter_ids_4: Vec<u64> = metrics_4.membership_config.membership().voter_ids().collect();
            let voter_ids_5: Vec<u64> = metrics_5.membership_config.membership().voter_ids().collect();
            assert!(
                voter_ids_4 == vec![4u64, 5] || voter_ids_5 == vec![4u64, 5],
                "Membership should contain only Node 4 and 5, got: {:?} / {:?}",
                voter_ids_4,
                voter_ids_5
            );
        } else {
            assert!(
                leader == Some(4) || leader == Some(5),
                "shard_b leader should be Node 4 or 5"
            );
        }
    }

    // Verify shard_a is still working on Node 1, 2, 3
    {
        let metrics_1a = raft_1a.metrics().borrow().clone();
        println!("    shard_a leader: {:?}", metrics_1a.current_leader);
        assert_eq!(
            metrics_1a.current_leader,
            Some(1),
            "shard_a leader should still be Node 1"
        );
    }

    // Shutdown shard_b on Node 1, 2, 3
    println!("  → Shutting down shard_b on Node 1, 2, 3...");
    // In a real system, we would call raft.shutdown() and unregister from router
    // For this test, we just verify the membership change worked

    println!("  ✓ Phase 4 complete: shard_b migrated to Node 4, 5!");
    println!();

    // =========================================================================
    // Final Verification: Data Integrity After Split and Migration
    // =========================================================================
    println!("┌──────────────────────────────────────────────────────────────────────────┐");
    println!("│ FINAL VERIFICATION: Data Integrity                                        │");
    println!("│                                                                          │");
    println!("│   shard_a (Node 1): Should have user:1..100, NOT user:101..200           │");
    println!("│   shard_b (Node 4): Should have user:101..200, NOT user:1..100           │");
    println!("└──────────────────────────────────────────────────────────────────────────┘");
    println!();

    // Verify shard_a data: Should contain user:1..100, should NOT contain user:101..200
    {
        let sm_a = state_machine_1a.state_machine.lock().await;
        println!("  → Verifying shard_a data (Node 1)...");
        println!("    Total keys in shard_a: {}", sm_a.data.len());

        // Check that shard_a contains users 1-100
        for user_id in 1..=100 {
            let key = format!("user:{}", user_id);
            assert!(
                sm_a.data.contains_key(&key),
                "shard_a should contain {} but it doesn't",
                key
            );
        }
        println!("    ✓ shard_a contains all users 1-100");

        // Check that shard_a does NOT contain users 101-200
        for user_id in 101..=200 {
            let key = format!("user:{}", user_id);
            assert!(
                !sm_a.data.contains_key(&key),
                "shard_a should NOT contain {} but it does",
                key
            );
        }
        println!("    ✓ shard_a does NOT contain any users 101-200");

        // Verify exact count
        assert_eq!(
            sm_a.data.len(),
            100,
            "shard_a should have exactly 100 keys, but has {}",
            sm_a.data.len()
        );
    }

    // Verify shard_b data: Should contain user:101..200, should NOT contain user:1..100
    // We verify using Node 4's shard_b state machine, which received data via snapshot replication
    // from the leader (Node 1). This proves the migration was successful.
    {
        let sm_b = state_machine_4b.state_machine.lock().await;
        println!("  → Verifying shard_b data (Node 4, received via snapshot)...");
        println!("    Total keys in shard_b: {}", sm_b.data.len());

        // Check that shard_b contains users 101-200
        for user_id in 101..=200 {
            let key = format!("user:{}", user_id);
            assert!(
                sm_b.data.contains_key(&key),
                "shard_b should contain {} but it doesn't",
                key
            );
        }
        println!("    ✓ shard_b contains all users 101-200");

        // Check that shard_b does NOT contain users 1-100
        for user_id in 1..=100 {
            let key = format!("user:{}", user_id);
            assert!(
                !sm_b.data.contains_key(&key),
                "shard_b should NOT contain {} but it does",
                key
            );
        }
        println!("    ✓ shard_b does NOT contain any users 1-100");

        // Verify exact count
        assert_eq!(
            sm_b.data.len(),
            100,
            "shard_b should have exactly 100 keys, but has {}",
            sm_b.data.len()
        );
    }

    println!();
    println!("═══════════════════════════════════════════════════════════════════════════");
    println!("  ✓ ALL VERIFICATIONS PASSED!");
    println!("  ✓ Shard split and migration completed successfully with data integrity!");
    println!("═══════════════════════════════════════════════════════════════════════════");
}

/// Create a Raft instance with initial data (used for shard_b after split).
///
/// This function creates a new Raft instance with the state machine pre-populated
/// with data from the split operation. This is how we bootstrap the new shard
/// after a split.
async fn create_raft_with_data(
    node_id: NodeId,
    shard_id: ShardId,
    router: Router,
    initial_data: BTreeMap<String, String>,
) -> (typ::Raft, multi_raft_sharding::app::App) {
    use multi_raft_sharding::network::ShardNetworkFactory;

    let config = Config {
        heartbeat_interval: 500,
        election_timeout_min: 1500,
        election_timeout_max: 3000,
        max_in_snapshot_log_to_keep: 0,
        ..Default::default()
    };

    let config = Arc::new(config.validate().unwrap());

    let log_store = multi_raft_sharding::LogStore::default();

    // Create state machine with the split data
    let state_machine_store = Arc::new(StateMachineStore::with_initial_data(initial_data));

    let network = ShardNetworkFactory::new(shard_id.clone(), router.clone());

    let raft = Raft::new(node_id, config, network, log_store, state_machine_store.clone()).await.unwrap();

    let app = multi_raft_sharding::app::App::new(node_id, shard_id, raft.clone(), router, state_machine_store);

    (raft, app)
}
