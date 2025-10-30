use std::time::Duration;

use client_http::ExampleClient;
use maplit::btreeset;
use raft_kv_memstore::start_example_raft_node;
use raft_kv_memstore::store::Request;
use raft_kv_memstore::TypeConfig;

/// Test follower read functionality
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_follower_read() -> Result<(), Box<dyn std::error::Error>> {
    fn get_addr(node_id: u64) -> String {
        format!("127.0.0.1:2800{}", node_id)
    }

    // Start 3 raft nodes
    let _h1 = tokio::spawn(start_example_raft_node(1, get_addr(1)));
    let _h2 = tokio::spawn(start_example_raft_node(2, get_addr(2)));
    let _h3 = tokio::spawn(start_example_raft_node(3, get_addr(3)));

    // Wait for servers to start
    tokio::time::sleep(Duration::from_millis(1000)).await;

    let leader = ExampleClient::<TypeConfig>::new(1, get_addr(1));

    // Initialize cluster
    println!("=== init single node cluster");
    leader.init().await??;

    // Wait for leader election
    println!("=== wait for leader election");
    loop {
        let metrics = leader.metrics().await?;
        if metrics.current_leader == Some(1) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // Add learners
    println!("=== add learners");
    leader.add_learner((2, get_addr(2))).await??;
    leader.add_learner((3, get_addr(3))).await??;

    // Change membership
    println!("=== change membership");
    leader.change_membership(&btreeset! {1,2,3}).await??;

    // Write some data
    println!("=== write test_key=test_value");
    leader
        .write(&Request::Set {
            key: "test_key".to_string(),
            value: "test_value".to_string(),
        })
        .await??;

    // Wait for replication
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Test follower read on node 2
    println!("=== follower_read on node 2");
    let client2 = ExampleClient::<TypeConfig>::new(2, get_addr(2));
    let result = client2.follower_read(&"test_key".to_string()).await?;
    let value = result.expect("follower_read should succeed");
    println!("=== follower_read returned: {}", value);
    assert_eq!("test_value", value, "follower read should return the correct value");

    // Test follower read on node 3
    println!("=== follower_read on node 3");
    let client3 = ExampleClient::<TypeConfig>::new(3, get_addr(3));
    let result = client3.follower_read(&"test_key".to_string()).await?;
    let value = result.expect("follower_read should succeed");
    println!("=== follower_read returned: {}", value);
    assert_eq!("test_value", value, "follower read should return the correct value");

    // Test with non-existent key
    println!("=== follower_read on node 2 with non-existent key");
    let result = client2.follower_read(&"non_existent".to_string()).await?;
    let value = result.expect("follower_read should succeed");
    println!("=== follower_read returned: {}", value);
    assert_eq!("", value, "non-existent key should return empty string");

    println!("=== test passed!");
    Ok(())
}
