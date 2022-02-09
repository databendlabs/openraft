use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyerror::AnyError;
use example_raft_key_value::client::ExampleClient;
use example_raft_key_value::start_example_raft_node;
use example_raft_key_value::store::ExampleRequest;
use maplit::btreeset;
use openraft::error::NodeNotFound;
use tokio::runtime::Runtime;

/// Setup a cluster of 3 nodes.
/// Write to it and read from it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_cluster() -> anyhow::Result<()> {
    // --- The client itself does not store addresses for all nodes, but just node id.
    //     Thus we need a supporting component to provide mapping from node id to node address.
    //     This is only used by the client. A raft node in this example stores node addresses in its store.

    let get_addr = |node_id| {
        let addr = match node_id {
            1 => "127.0.0.1:21001".to_string(),
            2 => "127.0.0.1:21002".to_string(),
            3 => "127.0.0.1:21003".to_string(),
            _ => {
                return Err(NodeNotFound {
                    node_id,
                    source: AnyError::error("node not found"),
                });
            }
        };
        Ok(addr)
    };
    let get_addr = Arc::new(get_addr);

    // --- Start 3 raft node in 3 threads.

    let _h1 = thread::spawn(|| {
        let rt = Runtime::new().unwrap();
        let x = rt.block_on(async move { start_example_raft_node(1, "127.0.0.1:21001".to_string()).await });
        println!("x: {:?}", x);
    });

    let _h2 = thread::spawn(|| {
        let rt = Runtime::new().unwrap();
        let x = rt.block_on(async move { start_example_raft_node(2, "127.0.0.1:21002".to_string()).await });
        println!("x: {:?}", x);
    });

    let _h3 = thread::spawn(|| {
        let rt = Runtime::new().unwrap();
        let x = rt.block_on(async move { start_example_raft_node(3, "127.0.0.1:21003".to_string()).await });
        println!("x: {:?}", x);
    });

    // --- Create a client to the first node, as a control handle to the cluster.

    let client = ExampleClient::new(1, get_addr.clone());

    // --- 1. Initialize the target node as a cluster of only one node.
    //        After init(), the single node cluster will be fully functional.

    client.init().await?;

    let m = client.metrics().await?;
    println!("metrics after init: {:?}", m);

    // --- 2. Before adding more members to the cluster, first we need to let the cluster know the address of every node
    //        to add.
    //        This is done with a regular raft protocol: replicate a log that contains node info and then apply the log
    //        to state machine.
    //        Then `RaftNetwork` will be able to read node address from its store.

    let x = client
        .write(&ExampleRequest::AddNode {
            id: 1,
            addr: "127.0.0.1:21001".to_string(),
        })
        .await?;
    println!("add node 1 res: {:?}", x);

    let x = client
        .write(&ExampleRequest::AddNode {
            id: 2,
            addr: "127.0.0.1:21002".to_string(),
        })
        .await?;
    println!("add node 2 res: {:?}", x);

    let x = client
        .write(&ExampleRequest::AddNode {
            id: 3,
            addr: "127.0.0.1:21003".to_string(),
        })
        .await?;
    println!("add node 3 res: {:?}", x);

    let x = client.list_nodes().await?;
    println!("list-nodes: {:?}", x);

    // --- 3. Add node 2 and 3 to the cluster as `Learner`, to let them start to receive log replication from the
    //        leader.

    let x = client.add_learner(&2).await?;
    println!("add-learner 2: {:?}", x);

    let x = client.add_learner(&3).await?;
    println!("add-learner 3: {:?}", x);

    // --- 4. Turn the two learners to members. A member node can vote or elect itself as leader.

    let x = client.change_membership(&btreeset! {1,2,3}).await?;
    println!("change-membership to 1,2,3: {:?}", x);

    // --- After change-membership, some cluster state will be seen in the metrics.
    //
    // ```text
    // metrics: RaftMetrics {
    //   current_leader: Some(1),
    //   membership_config: EffectiveMembership {
    //        log_id: LogId { leader_id: LeaderId { term: 1, node_id: 1 }, index: 8 },
    //        membership: Membership { learners: {}, configs: [{1, 2, 3}], all_members: {1, 2, 3} }
    //   },
    //   leader_metrics: Some(LeaderMetrics { replication: {
    //     2: ReplicationMetrics { matched: Some(LogId { leader_id: LeaderId { term: 1, node_id: 1 }, index: 7 }) },
    //     3: ReplicationMetrics { matched: Some(LogId { leader_id: LeaderId { term: 1, node_id: 1 }, index: 8 }) }} })
    // }
    // ```

    let x = client.metrics().await?;
    println!("metrics after change-member: {:?}", x);

    // --- Try to write some application data through the leader.

    let x = client
        .write(&ExampleRequest::Set {
            key: "foo".to_string(),
            value: "bar".to_string(),
        })
        .await?;
    println!("write `foo` res: {:?}", x);

    // --- Wait for a while to let the replication get done.

    tokio::time::sleep(Duration::from_millis(200)).await;

    // --- Read it on every node.

    let x = client.read(&("foo".to_string())).await?;
    println!("read `foo` on node 1: {:?}", x);

    let client2 = ExampleClient::new(2, get_addr.clone());
    let x = client2.read(&("foo".to_string())).await?;
    println!("read `foo` on node 2: {:?}", x);

    let client3 = ExampleClient::new(3, get_addr.clone());
    let x = client3.read(&("foo".to_string())).await?;
    println!("read `foo` on node 3: {:?}", x);

    // --- A write to non-leader will be automatically forwarded to a known leader

    let x = client2
        .write(&ExampleRequest::Set {
            key: "foo".to_string(),
            value: "wow".to_string(),
        })
        .await?;
    println!("write `foo` to node-2 res: {:?}", x);

    tokio::time::sleep(Duration::from_millis(200)).await;

    // --- Read it on every node.

    let x = client.read(&("foo".to_string())).await?;
    println!("read `foo` on node 1: {:?}", x);

    let client2 = ExampleClient::new(2, get_addr.clone());
    let x = client2.read(&("foo".to_string())).await?;
    println!("read `foo` on node 2: {:?}", x);

    let client3 = ExampleClient::new(3, get_addr.clone());
    let x = client3.read(&("foo".to_string())).await?;
    println!("read `foo` on node 3: {:?}", x);

    Ok(())
}
