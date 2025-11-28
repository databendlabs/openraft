//! Integration test for Multi-Raft KV store with 3 groups.
//!
//! This test demonstrates:
//! - Running 3 independent Raft groups ("users", "orders", "products") on 2 nodes
//! - Each group runs its own independent consensus
//! - Groups share the same network infrastructure (Router)
//! - Snapshot replication works independently for each group

use std::backtrace::Backtrace;
use std::collections::BTreeMap;
use std::panic::PanicHookInfo;
use std::time::Duration;

use multi_raft_kv::groups;
use multi_raft_kv::new_raft;
use multi_raft_kv::router::Router;
use multi_raft_kv::store::Request;
use multi_raft_kv::typ;
use multi_raft_kv::GroupId;
use openraft::BasicNode;
use tokio::task;
use tokio::task::LocalSet;
use tracing_subscriber::EnvFilter;

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

/// Test Multi-Raft cluster with 3 groups and 2 nodes.
///
/// Architecture:
/// ```text
/// Node 1:                    Node 2:
/// +------------------+       +------------------+
/// | Group: users     |  <->  | Group: users     |
/// | Group: orders    |  <->  | Group: orders    |
/// | Group: products  |  <->  | Group: products  |
/// +------------------+       +------------------+
/// ```
///
/// Each group:
/// - Node 1 is initialized as the leader
/// - Node 2 is added as a learner
/// - Data is written to each group independently
/// - Snapshot replication happens per-group
#[tokio::test]
async fn test_multi_raft_cluster() {
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

    // Shared router for all groups and nodes
    let router = Router::new();

    let local = LocalSet::new();

    // Create Raft instances for all 3 groups on both nodes
    let mut node1_rafts: Vec<typ::Raft> = Vec::new();
    let mut node2_rafts: Vec<typ::Raft> = Vec::new();
    let mut apps = Vec::new();

    for group_id in groups::all() {
        // Node 1
        let (raft1, app1) = new_raft(1, group_id.clone(), router.clone()).await;
        node1_rafts.push(raft1);
        apps.push(app1);

        // Node 2
        let (raft2, app2) = new_raft(2, group_id, router.clone()).await;
        node2_rafts.push(raft2);
        apps.push(app2);
    }

    local
        .run_until(async move {
            // Spawn all app handlers
            for app in apps {
                task::spawn_local(app.run());
            }

            run_test(&node1_rafts, &node2_rafts).await;
        })
        .await;
}

async fn run_test(node1_rafts: &[typ::Raft], node2_rafts: &[typ::Raft]) {
    // Wait for servers to start up
    tokio::time::sleep(Duration::from_millis(200)).await;

    let group_ids = groups::all();

    // =========================================================================
    // Initialize each group with node 1 as leader
    // =========================================================================
    println!("=== Initializing 3 Raft groups");
    for (i, group_id) in group_ids.iter().enumerate() {
        let raft = &node1_rafts[i];
        let mut nodes = BTreeMap::new();
        nodes.insert(1u64, BasicNode { addr: "".to_string() });
        raft.initialize(nodes).await.unwrap();
        println!("  Initialized group: {}", group_id);
    }

    tokio::time::sleep(Duration::from_millis(500)).await;

    // =========================================================================
    // Write data to each group (different data per group)
    // =========================================================================
    println!("\n=== Writing data to each group");

    // Users group
    {
        let raft = &node1_rafts[0];
        raft.client_write(Request::set("user:1", "Alice")).await.unwrap();
        raft.client_write(Request::set("user:2", "Bob")).await.unwrap();
        println!("  Users: wrote user:1=Alice, user:2=Bob");
    }

    // Orders group
    {
        let raft = &node1_rafts[1];
        raft.client_write(Request::set("order:1001", "pending")).await.unwrap();
        raft.client_write(Request::set("order:1002", "shipped")).await.unwrap();
        println!("  Orders: wrote order:1001=pending, order:1002=shipped");
    }

    // Products group
    {
        let raft = &node1_rafts[2];
        raft.client_write(Request::set("product:A", "Widget")).await.unwrap();
        raft.client_write(Request::set("product:B", "Gadget")).await.unwrap();
        println!("  Products: wrote product:A=Widget, product:B=Gadget");
    }

    // =========================================================================
    // Take snapshots for each group
    // =========================================================================
    println!("\n=== Taking snapshots for each group");
    for (i, group_id) in group_ids.iter().enumerate() {
        let raft = &node1_rafts[i];
        raft.trigger().snapshot().await.unwrap();
        println!("  Triggered snapshot for group: {}", group_id);
    }

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Verify snapshots
    for (i, group_id) in group_ids.iter().enumerate() {
        let metrics = node1_rafts[i].metrics().borrow().clone();
        println!(
            "  Group {} snapshot index: {:?}",
            group_id,
            metrics.snapshot.map(|x| x.index)
        );
        assert!(metrics.snapshot.is_some(), "Group {} should have snapshot", group_id);
    }

    // =========================================================================
    // Add node 2 as learner to each group
    // =========================================================================
    println!("\n=== Adding node 2 as learner to each group");
    for (i, group_id) in group_ids.iter().enumerate() {
        let raft = &node1_rafts[i];
        let node = BasicNode { addr: "".to_string() };
        raft.add_learner(2, node, true).await.unwrap();
        println!("  Added learner to group: {}", group_id);
    }

    // Wait for snapshot replication
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // =========================================================================
    // Verify node 2 received snapshots for all groups
    // =========================================================================
    println!("\n=== Verifying node 2 received snapshots");
    for (i, group_id) in group_ids.iter().enumerate() {
        let metrics = node2_rafts[i].metrics().borrow().clone();
        println!(
            "  Group {} on node 2: snapshot={:?}, last_applied={:?}",
            group_id,
            metrics.snapshot.map(|x| x.index),
            metrics.last_applied.map(|x| x.index)
        );
        assert!(
            metrics.snapshot.is_some(),
            "Node 2 group {} should have received snapshot",
            group_id
        );
    }

    // =========================================================================
    // Verify data isolation - each group has only its own data
    // =========================================================================
    println!("\n=== Verifying data isolation between groups");

    // Check users group has user data
    {
        let snapshot = node2_rafts[0].get_snapshot().await.unwrap().unwrap();
        assert!(snapshot.snapshot.data.contains_key("user:1"));
        assert!(snapshot.snapshot.data.contains_key("user:2"));
        assert!(!snapshot.snapshot.data.contains_key("order:1001"));
        assert!(!snapshot.snapshot.data.contains_key("product:A"));
        println!("  Users group: ✓ only contains user data");
    }

    // Check orders group has order data
    {
        let snapshot = node2_rafts[1].get_snapshot().await.unwrap().unwrap();
        assert!(snapshot.snapshot.data.contains_key("order:1001"));
        assert!(snapshot.snapshot.data.contains_key("order:1002"));
        assert!(!snapshot.snapshot.data.contains_key("user:1"));
        assert!(!snapshot.snapshot.data.contains_key("product:A"));
        println!("  Orders group: ✓ only contains order data");
    }

    // Check products group has product data
    {
        let snapshot = node2_rafts[2].get_snapshot().await.unwrap().unwrap();
        assert!(snapshot.snapshot.data.contains_key("product:A"));
        assert!(snapshot.snapshot.data.contains_key("product:B"));
        assert!(!snapshot.snapshot.data.contains_key("user:1"));
        assert!(!snapshot.snapshot.data.contains_key("order:1001"));
        println!("  Products group: ✓ only contains product data");
    }

    println!("\n=== All tests passed! ===");
    println!("Summary:");
    println!("  - 3 independent Raft groups running on 2 nodes");
    println!("  - Each group maintains separate consensus");
    println!("  - Snapshot replication works per-group");
    println!("  - Data is properly isolated between groups");
}

/// Helper to get a group's Raft instance from a list by group ID
#[allow(dead_code)]
fn get_group_raft<'a>(rafts: &'a [typ::Raft], group_ids: &[GroupId], target: &str) -> &'a typ::Raft {
    let idx = group_ids.iter().position(|g| g == target).unwrap();
    &rafts[idx]
}

// =============================================================================
// Test: Leader Distribution Across Nodes
// =============================================================================

/// Test Multi-Raft cluster with 3 groups distributed across 3 nodes.
///
/// This test demonstrates that different Raft groups can have their leaders
/// on different physical nodes, which is a key feature of Multi-Raft for
/// load balancing and avoiding single-node hot spots.
///
/// Architecture:
/// ```text
/// +-------------------+     +-------------------+     +-------------------+
/// |      Node 1       |     |      Node 2       |     |      Node 3       |
/// +-------------------+     +-------------------+     +-------------------+
/// | users:  LEADER ★  |     | users:  Follower  |     | users:  Follower  |
/// | orders: Follower  |     | orders: LEADER ★  |     | orders: Follower  |
/// | products: Follower|     | products: Follower|     | products: LEADER ★|
/// +-------------------+     +-------------------+     +-------------------+
/// ```
///
/// Each group has its leader on a different node, distributing the write load.
#[tokio::test]
async fn test_leader_distribution() {
    std::panic::set_hook(Box::new(|panic| {
        log_panic(panic);
    }));

    // Shared router for all groups and nodes
    let router = Router::new();

    let local = LocalSet::new();

    // Create Raft instances for all 3 groups on all 3 nodes
    // node_rafts[node_idx][group_idx]
    let mut node1_rafts: Vec<typ::Raft> = Vec::new();
    let mut node2_rafts: Vec<typ::Raft> = Vec::new();
    let mut node3_rafts: Vec<typ::Raft> = Vec::new();
    let mut apps = Vec::new();

    for group_id in groups::all() {
        // Node 1
        let (raft1, app1) = new_raft(1, group_id.clone(), router.clone()).await;
        node1_rafts.push(raft1);
        apps.push(app1);

        // Node 2
        let (raft2, app2) = new_raft(2, group_id.clone(), router.clone()).await;
        node2_rafts.push(raft2);
        apps.push(app2);

        // Node 3
        let (raft3, app3) = new_raft(3, group_id, router.clone()).await;
        node3_rafts.push(raft3);
        apps.push(app3);
    }

    local
        .run_until(async move {
            // Spawn all app handlers
            for app in apps {
                task::spawn_local(app.run());
            }

            run_leader_distribution_test(&node1_rafts, &node2_rafts, &node3_rafts).await;
        })
        .await;
}

async fn run_leader_distribution_test(node1_rafts: &[typ::Raft], node2_rafts: &[typ::Raft], node3_rafts: &[typ::Raft]) {
    // Wait for servers to start up
    tokio::time::sleep(Duration::from_millis(200)).await;

    // group_ids = ["users", "orders", "products"]
    let _group_ids = groups::all();

    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║   Multi-Raft Leader Distribution Test                            ║");
    println!("║   Demonstrating leaders on different nodes per group             ║");
    println!("╚══════════════════════════════════════════════════════════════════╝\n");

    // =========================================================================
    // Initialize each group with a DIFFERENT leader node
    // - users   -> Leader on Node 1
    // - orders  -> Leader on Node 2
    // - products -> Leader on Node 3
    // =========================================================================
    println!("=== Initializing 3 groups with leaders on DIFFERENT nodes ===\n");

    // Users group: Initialize on Node 1 (Node 1 will be leader)
    {
        let mut nodes = BTreeMap::new();
        nodes.insert(1u64, BasicNode { addr: "".to_string() });
        node1_rafts[0].initialize(nodes).await.unwrap();
        println!("  ★ Group 'users'    initialized on Node 1 → Node 1 is LEADER");
    }

    // Orders group: Initialize on Node 2 (Node 2 will be leader)
    {
        let mut nodes = BTreeMap::new();
        nodes.insert(2u64, BasicNode { addr: "".to_string() });
        node2_rafts[1].initialize(nodes).await.unwrap();
        println!("  ★ Group 'orders'   initialized on Node 2 → Node 2 is LEADER");
    }

    // Products group: Initialize on Node 3 (Node 3 will be leader)
    {
        let mut nodes = BTreeMap::new();
        nodes.insert(3u64, BasicNode { addr: "".to_string() });
        node3_rafts[2].initialize(nodes).await.unwrap();
        println!("  ★ Group 'products' initialized on Node 3 → Node 3 is LEADER");
    }

    tokio::time::sleep(Duration::from_millis(500)).await;

    // =========================================================================
    // Verify leader distribution
    // =========================================================================
    println!("\n=== Verifying Leader Distribution ===\n");

    // Check users group - leader should be on Node 1
    {
        let metrics = node1_rafts[0].metrics().borrow().clone();
        let is_leader = metrics.current_leader == Some(1);
        println!(
            "  Group 'users':    current_leader={:?}, Node 1 is_leader={}",
            metrics.current_leader, is_leader
        );
        assert_eq!(metrics.current_leader, Some(1), "Users group leader should be Node 1");
    }

    // Check orders group - leader should be on Node 2
    {
        let metrics = node2_rafts[1].metrics().borrow().clone();
        let is_leader = metrics.current_leader == Some(2);
        println!(
            "  Group 'orders':   current_leader={:?}, Node 2 is_leader={}",
            metrics.current_leader, is_leader
        );
        assert_eq!(metrics.current_leader, Some(2), "Orders group leader should be Node 2");
    }

    // Check products group - leader should be on Node 3
    {
        let metrics = node3_rafts[2].metrics().borrow().clone();
        let is_leader = metrics.current_leader == Some(3);
        println!(
            "  Group 'products': current_leader={:?}, Node 3 is_leader={}",
            metrics.current_leader, is_leader
        );
        assert_eq!(
            metrics.current_leader,
            Some(3),
            "Products group leader should be Node 3"
        );
    }

    // =========================================================================
    // Write data to each group (must write to the leader!)
    // =========================================================================
    println!("\n=== Writing data to each group via their respective leaders ===\n");

    // Write to users group via Node 1 (leader)
    {
        node1_rafts[0].client_write(Request::set("user:1", "Alice")).await.unwrap();
        println!("  → Wrote to 'users' group via Node 1 (leader): user:1=Alice");
    }

    // Write to orders group via Node 2 (leader)
    {
        node2_rafts[1].client_write(Request::set("order:1001", "pending")).await.unwrap();
        println!("  → Wrote to 'orders' group via Node 2 (leader): order:1001=pending");
    }

    // Write to products group via Node 3 (leader)
    {
        node3_rafts[2].client_write(Request::set("product:A", "Widget")).await.unwrap();
        println!("  → Wrote to 'products' group via Node 3 (leader): product:A=Widget");
    }

    // =========================================================================
    // Add other nodes as learners to each group
    // =========================================================================
    println!("\n=== Adding learners to form 3-node clusters for each group ===\n");

    // Users: Add Node 2 and Node 3 as learners
    {
        let node = BasicNode { addr: "".to_string() };
        node1_rafts[0].add_learner(2, node.clone(), true).await.unwrap();
        node1_rafts[0].add_learner(3, node, true).await.unwrap();
        println!("  Group 'users': Added Node 2, Node 3 as learners (leader=Node 1)");
    }

    // Orders: Add Node 1 and Node 3 as learners
    {
        let node = BasicNode { addr: "".to_string() };
        node2_rafts[1].add_learner(1, node.clone(), true).await.unwrap();
        node2_rafts[1].add_learner(3, node, true).await.unwrap();
        println!("  Group 'orders': Added Node 1, Node 3 as learners (leader=Node 2)");
    }

    // Products: Add Node 1 and Node 2 as learners
    {
        let node = BasicNode { addr: "".to_string() };
        node3_rafts[2].add_learner(1, node.clone(), true).await.unwrap();
        node3_rafts[2].add_learner(2, node, true).await.unwrap();
        println!("  Group 'products': Added Node 1, Node 2 as learners (leader=Node 3)");
    }

    // Wait for replication
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // =========================================================================
    // Verify data was replicated to all nodes
    // =========================================================================
    println!("\n=== Verifying data replication ===\n");

    // Verify users data replicated to Node 2
    {
        let metrics = node2_rafts[0].metrics().borrow().clone();
        println!(
            "  'users' on Node 2: last_applied={:?}",
            metrics.last_applied.map(|x| x.index)
        );
        assert!(
            metrics.last_applied.is_some(),
            "Users data should be replicated to Node 2"
        );
    }

    // Verify orders data replicated to Node 1
    {
        let metrics = node1_rafts[1].metrics().borrow().clone();
        println!(
            "  'orders' on Node 1: last_applied={:?}",
            metrics.last_applied.map(|x| x.index)
        );
        assert!(
            metrics.last_applied.is_some(),
            "Orders data should be replicated to Node 1"
        );
    }

    // Verify products data replicated to Node 1
    {
        let metrics = node1_rafts[2].metrics().borrow().clone();
        println!(
            "  'products' on Node 1: last_applied={:?}",
            metrics.last_applied.map(|x| x.index)
        );
        assert!(
            metrics.last_applied.is_some(),
            "Products data should be replicated to Node 1"
        );
    }
}
