use std::backtrace::Backtrace;
use std::collections::BTreeMap;
use std::panic::PanicInfo;
use std::thread;
use std::time::Duration;

use maplit::btreemap;
use maplit::btreeset;
use openraft::BasicNode;
use raft_kv_memstore::client::ExampleClient;
use raft_kv_memstore::start_example_raft_node;
use raft_kv_memstore::store::Request;
use tokio::runtime::Runtime;
use tracing_subscriber::EnvFilter;

pub fn log_panic(panic: &PanicInfo) {
    let backtrace = {
        format!("{:?}", Backtrace::force_capture())
        // #[cfg(feature = "bt")]
        // {
        //     format!("{:?}", Backtrace::force_capture())
        // }
        //
        // #[cfg(not(feature = "bt"))]
        // {
        //     "backtrace is disabled without --features 'bt'".to_string()
        // }
    };

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

/// Setup a cluster of 3 nodes.
/// Write to it and read from it.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn test_cluster() -> anyhow::Result<()> {
    // --- The client itself does not store addresses for all nodes, but just node id.
    //     Thus we need a supporting component to provide mapping from node id to node address.
    //     This is only used by the client. A raft node in this example stores node addresses in its
    // store.

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

    let get_addr = |node_id| {
        let addr = match node_id {
            1 => "127.0.0.1:21001".to_string(),
            2 => "127.0.0.1:21002".to_string(),
            3 => "127.0.0.1:21003".to_string(),
            _ => {
                return Err(anyhow::anyhow!("node {} not found", node_id));
            }
        };
        Ok(addr)
    };

    // --- Start 3 raft node in 3 threads.

    let _h1 = thread::spawn(|| {
        let rt = Runtime::new().unwrap();
        let x = rt.block_on(start_example_raft_node(1, "127.0.0.1:21001".to_string()));
        println!("x: {:?}", x);
    });

    let _h2 = thread::spawn(|| {
        let rt = Runtime::new().unwrap();
        let x = rt.block_on(start_example_raft_node(2, "127.0.0.1:21002".to_string()));
        println!("x: {:?}", x);
    });

    let _h3 = thread::spawn(|| {
        let rt = Runtime::new().unwrap();
        let x = rt.block_on(start_example_raft_node(3, "127.0.0.1:21003".to_string()));
        println!("x: {:?}", x);
    });

    // Wait for server to start up.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // --- Create a client to the first node, as a control handle to the cluster.

    let client = ExampleClient::new(1, get_addr(1)?);

    // --- 1. Initialize the target node as a cluster of only one node.
    //        After init(), the single node cluster will be fully functional.

    println!("=== init single node cluster");
    client.init().await?;

    println!("=== metrics after init");
    let _x = client.metrics().await?;

    // --- 2. Add node 2 and 3 to the cluster as `Learner`, to let them start to receive log replication
    // from the        leader.

    println!("=== add-learner 2");
    let _x = client.add_learner((2, get_addr(2)?)).await?;

    println!("=== add-learner 3");
    let _x = client.add_learner((3, get_addr(3)?)).await?;

    println!("=== metrics after add-learner");
    let x = client.metrics().await?;

    assert_eq!(&vec![btreeset![1]], x.membership_config.membership().get_joint_config());

    let nodes_in_cluster =
        x.membership_config.nodes().map(|(nid, node)| (*nid, node.clone())).collect::<BTreeMap<_, _>>();
    assert_eq!(
        btreemap! {
            1 => BasicNode::new("127.0.0.1:21001"),
            2 => BasicNode::new("127.0.0.1:21002"),
            3 => BasicNode::new("127.0.0.1:21003"),
        },
        nodes_in_cluster
    );

    // --- 3. Turn the two learners to members. A member node can vote or elect itself as leader.

    println!("=== change-membership to 1,2,3");
    let _x = client.change_membership(&btreeset! {1,2,3}).await?;

    // --- After change-membership, some cluster state will be seen in the metrics.
    //
    // ```text
    // metrics: RaftMetrics {
    //   current_leader: Some(1),
    //   membership_config: EffectiveMembership {
    //        log_id: LogId { leader_id: LeaderId { term: 1, node_id: 1 }, index: 8 },
    //        membership: Membership { learners: {}, configs: [{1, 2, 3}] }
    //   },
    //   leader_metrics: Some(LeaderMetrics { replication: {
    //     2: ReplicationMetrics { matched: Some(LogId { leader_id: LeaderId { term: 1, node_id: 1 }, index: 7 }) },
    //     3: ReplicationMetrics { matched: Some(LogId { leader_id: LeaderId { term: 1, node_id: 1 }, index: 8 }) }} })
    // }
    // ```

    println!("=== metrics after change-member");
    let x = client.metrics().await?;
    assert_eq!(
        &vec![btreeset![1, 2, 3]],
        x.membership_config.membership().get_joint_config()
    );

    // --- Try to write some application data through the leader.

    println!("=== write `foo=bar`");
    let _x = client
        .write(&Request::Set {
            key: "foo".to_string(),
            value: "bar".to_string(),
        })
        .await?;

    // --- Wait for a while to let the replication get done.

    tokio::time::sleep(Duration::from_millis(1_000)).await;

    // --- Read it on every node.

    println!("=== read `foo` on node 1");
    let x = client.read(&("foo".to_string())).await?;
    assert_eq!("bar", x);

    println!("=== read `foo` on node 2");
    let client2 = ExampleClient::new(2, get_addr(2)?);
    let x = client2.read(&("foo".to_string())).await?;
    assert_eq!("bar", x);

    println!("=== read `foo` on node 3");
    let client3 = ExampleClient::new(3, get_addr(3)?);
    let x = client3.read(&("foo".to_string())).await?;
    assert_eq!("bar", x);

    // --- A write to non-leader will be automatically forwarded to a known leader

    println!("=== read `foo` on node 2");
    let _x = client2
        .write(&Request::Set {
            key: "foo".to_string(),
            value: "wow".to_string(),
        })
        .await?;

    tokio::time::sleep(Duration::from_millis(1_000)).await;

    // --- Read it on every node.

    println!("=== read `foo` on node 1");
    let x = client.read(&("foo".to_string())).await?;
    assert_eq!("wow", x);

    println!("=== read `foo` on node 2");
    let client2 = ExampleClient::new(2, get_addr(2)?);
    let x = client2.read(&("foo".to_string())).await?;
    assert_eq!("wow", x);

    println!("=== read `foo` on node 3");
    let client3 = ExampleClient::new(3, get_addr(3)?);
    let x = client3.read(&("foo".to_string())).await?;
    assert_eq!("wow", x);

    println!("=== consistent_read `foo` on node 1");
    let x = client.consistent_read(&("foo".to_string())).await?;
    assert_eq!("wow", x);

    println!("=== consistent_read `foo` on node 2 MUST return CheckIsLeaderError");
    let x = client2.consistent_read(&("foo".to_string())).await;
    match x {
        Err(e) => {
            let s = e.to_string();
            let expect_err:String = "error occur on remote peer 2: has to forward request to: Some(1), Some(BasicNode { addr: \"127.0.0.1:21001\" })".to_string();

            assert_eq!(s, expect_err);
        }
        Ok(_) => panic!("MUST return CheckIsLeaderError"),
    }

    // --- Remove node 1,2 from the cluster.

    println!("=== change-membership to 3, ");
    let _x = client.change_membership(&btreeset! {3}).await?;

    tokio::time::sleep(Duration::from_millis(8_000)).await;

    println!("=== metrics after change-membership to {{3}}");
    let x = client.metrics().await?;
    assert_eq!(&vec![btreeset![3]], x.membership_config.membership().get_joint_config());

    println!("=== write `foo=zoo` to node-3");
    let _x = client3
        .write(&Request::Set {
            key: "foo".to_string(),
            value: "zoo".to_string(),
        })
        .await?;

    println!("=== read `foo=zoo` to node-3");
    let got = client3.read(&"foo".to_string()).await?;
    assert_eq!("zoo", got);
    Ok(())
}
