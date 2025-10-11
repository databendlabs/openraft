use std::backtrace::Backtrace;
use std::collections::BTreeMap;
use std::panic::PanicHookInfo;
use std::thread;
use std::time::Duration;

use client_http::ExampleClient;
use maplit::btreemap;
use maplit::btreeset;
use openraft::BasicNode;
use raft_kv_rocksdb::start_example_raft_node;
use raft_kv_rocksdb::store::Request;
use raft_kv_rocksdb::TypeConfig;
use tokio::runtime::Handle;
use tracing_subscriber::EnvFilter;

pub fn log_panic(panic: &PanicHookInfo) {
    let backtrace = { format!("{:?}", Backtrace::force_capture()) };

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
async fn test_cluster() -> Result<(), Box<dyn std::error::Error>> {
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

    fn get_addr(node_id: u32) -> String {
        match node_id {
            1 => "127.0.0.1:31001".to_string(),
            2 => "127.0.0.1:31002".to_string(),
            3 => "127.0.0.1:31003".to_string(),
            _ => panic!("node not found"),
        }
    }

    // --- Start 3 raft node in 3 threads.
    let d1 = tempfile::TempDir::new()?;
    let d2 = tempfile::TempDir::new()?;
    let d3 = tempfile::TempDir::new()?;

    let handle = Handle::current();
    let handle_clone = handle.clone();
    let _h1 = thread::spawn(move || {
        let x = handle_clone.block_on(start_example_raft_node(1, d1.path(), get_addr(1)));
        println!("x: {:?}", x);
    });

    let handle_clone = handle.clone();
    let _h2 = thread::spawn(move || {
        let x = handle_clone.block_on(start_example_raft_node(2, d2.path(), get_addr(2)));
        println!("x: {:?}", x);
    });

    let _h3 = thread::spawn(move || {
        let x = handle.block_on(start_example_raft_node(3, d3.path(), get_addr(3)));
        println!("x: {:?}", x);
    });

    // Wait for server to start up.
    tokio::time::sleep(Duration::from_millis(3_000)).await;

    // --- Create a client to the first node, as a control handle to the cluster.

    let leader = ExampleClient::<TypeConfig>::new(1, get_addr(1));

    // --- 1. Initialize the target node as a cluster of only one node.
    //        After init(), the single node cluster will be fully functional.

    println!("=== init single node cluster");
    leader.init().await??;

    println!("=== get metrics after init, wait until leader is elected");
    loop {
        let metrics = leader.metrics().await?;
        if metrics.current_leader == Some(1) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    // --- 2. Add node 2 and 3 to the cluster as `Learner`, to let them start to receive log replication
    // from the        leader.

    println!("=== add-learner 2");
    leader.add_learner((2, get_addr(2))).await??;

    println!("=== add-learner 3");
    leader.add_learner((3, get_addr(3))).await??;

    println!("=== metrics after add-learner");
    let x = leader.metrics().await?;

    assert_eq!(&vec![btreeset![1]], x.membership_config.membership().get_joint_config());

    let nodes_in_cluster =
        x.membership_config.nodes().map(|(nid, node)| (*nid, node.clone())).collect::<BTreeMap<_, _>>();
    assert_eq!(
        btreemap! {
            1 => BasicNode::new(get_addr(1)),
            2 => BasicNode::new(get_addr(2)),
            3 => BasicNode::new(get_addr(3)),
        },
        nodes_in_cluster
    );

    // --- 3. Turn the two learners to members. A member node can vote or elect itself as leader.

    println!("=== change-membership to 1,2,3");
    leader.change_membership(&btreeset! {1,2,3}).await??;

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
    let x = leader.metrics().await?;
    assert_eq!(
        &vec![btreeset![1, 2, 3]],
        x.membership_config.membership().get_joint_config()
    );

    // --- Try to write some application data through the leader.

    println!("=== write `foo=bar`");
    leader
        .write(&Request::Set {
            key: "foo".to_string(),
            value: "bar".to_string(),
        })
        .await??;

    // --- Wait for a while to let the replication get done.

    tokio::time::sleep(Duration::from_millis(500)).await;

    // --- Read it on every node.

    println!("=== read `foo=bar` on node 1");
    let x = leader.read(&("foo".to_string())).await?;
    assert_eq!("bar", x);

    println!("=== read `foo=bar` on node 2");
    let client2 = ExampleClient::<TypeConfig>::new(2, get_addr(2));
    let x = client2.read(&("foo".to_string())).await?;
    assert_eq!("bar", x);

    println!("=== read `foo=bar` on node 3");
    let client3 = ExampleClient::<TypeConfig>::new(3, get_addr(3));
    let x = client3.read(&("foo".to_string())).await?;
    assert_eq!("bar", x);

    // --- A write to non-leader will be automatically forwarded to a known leader

    println!("=== write `foo=wow` on node 2");
    client2
        .write(&Request::Set {
            key: "foo".to_string(),
            value: "wow".to_string(),
        })
        .await??;

    tokio::time::sleep(Duration::from_millis(500)).await;

    // --- Read it on every node.

    println!("=== read `foo=wow` on node 1");
    let x = leader.read(&("foo".to_string())).await?;
    assert_eq!("wow", x);

    println!("=== read `foo=wow` on node 2");
    let client2 = ExampleClient::<TypeConfig>::new(2, get_addr(2));
    let x = client2.read(&("foo".to_string())).await?;
    assert_eq!("wow", x);

    println!("=== read `foo=wow` on node 3");
    let client3 = ExampleClient::<TypeConfig>::new(3, get_addr(3));
    let x = client3.read(&("foo".to_string())).await?;
    assert_eq!("wow", x);

    println!("=== linearizable_read `foo=wow` on node 1");
    let x = leader.linearizable_read(&("foo".to_string())).await??;
    assert_eq!("wow", x);

    println!("=== linearizable_read `foo=wow` on node 2 MUST return CheckIsLeaderError");
    let x = client2.linearizable_read(&("foo".to_string())).await?;
    println!("=== linearize_read on node 2 result: {:?}", x);
    match x {
        Err(e) => {
            let s = e.to_string();
            let expect_err: String =
                "has to forward request to: Some(1), Some(BasicNode { addr: \"127.0.0.1:31001\" })".to_string();

            assert_eq!(s, expect_err);
        }
        Ok(_) => panic!("MUST return CheckIsLeaderError"),
    }

    println!("=== linearizable_read_auto_forward `foo=wow` on node 2 returns value");
    let x = client2.linearizable_read_auto_forward(&("foo".to_string())).await?;
    assert_eq!(x.unwrap(), "wow");

    Ok(())
}
