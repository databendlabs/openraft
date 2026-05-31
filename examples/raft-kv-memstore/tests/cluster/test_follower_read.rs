use std::time::Duration;

use client_http::ExampleClient;
use maplit::btreeset;
use openraft::type_config::TypeConfigExt;
use raft_kv_memstore::TypeConfig;
use raft_kv_memstore::network::management::AddLearnerRequest;
use raft_kv_memstore::start_example_raft_node;

#[test]
fn test_follower_read() {
    TypeConfig::run(test_follower_read_inner()).unwrap();
}

async fn test_follower_read_inner() -> anyhow::Result<()> {
    fn api_addr(node_id: u64) -> String {
        format!("127.0.0.1:2800{}", node_id)
    }

    fn raft_addr(node_id: u64) -> String {
        format!("127.0.0.1:2900{}", node_id)
    }

    let _h1 = TypeConfig::spawn(start_example_raft_node(1, api_addr(1), raft_addr(1)));
    let _h2 = TypeConfig::spawn(start_example_raft_node(2, api_addr(2), raft_addr(2)));
    let _h3 = TypeConfig::spawn(start_example_raft_node(3, api_addr(3), raft_addr(3)));

    TypeConfig::sleep(Duration::from_millis(1000)).await;

    let leader = ExampleClient::<TypeConfig>::new(1, api_addr(1));

    println!("=== init single node cluster");
    leader.init().await??;

    println!("=== wait for leader election");
    loop {
        let metrics = leader.metrics().await?;
        if metrics.current_leader == Some(1) {
            break;
        }
        TypeConfig::sleep(Duration::from_millis(200)).await;
    }

    println!("=== add learners");
    leader
        .add_learner(&AddLearnerRequest {
            node_id: 2,
            api_addr: api_addr(2),
            raft_addr: raft_addr(2),
        })
        .await??;
    leader
        .add_learner(&AddLearnerRequest {
            node_id: 3,
            api_addr: api_addr(3),
            raft_addr: raft_addr(3),
        })
        .await??;

    println!("=== change membership");
    leader.change_membership(&btreeset! {1,2,3}).await??;

    println!("=== write test_key=test_value");
    leader
        .write(&types_kv::Request::Set {
            key: "test_key".to_string(),
            value: "test_value".to_string(),
        })
        .await??;

    TypeConfig::sleep(Duration::from_millis(500)).await;

    println!("=== follower_read on node 2");
    let client2 = ExampleClient::<TypeConfig>::new(2, api_addr(2));
    let value = client2.follower_read(&"test_key".to_string()).await??;
    assert_eq!("test_value", value);

    println!("=== follower_read on node 3");
    let client3 = ExampleClient::<TypeConfig>::new(3, api_addr(3));
    let value = client3.follower_read(&"test_key".to_string()).await??;
    assert_eq!("test_value", value);

    println!("=== follower_read on node 2 with non-existent key");
    let value = client2.follower_read(&"non_existent".to_string()).await??;
    assert_eq!("", value);

    Ok(())
}
