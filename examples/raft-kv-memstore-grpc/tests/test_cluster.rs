#![allow(clippy::uninlined_format_args)]
use std::backtrace::Backtrace;
use std::panic::PanicHookInfo;
use std::thread;
use std::time::Duration;

use maplit::btreemap;
use raft_kv_memstore_grpc::app::start_raft_app;
use raft_kv_memstore_grpc::protobuf as pb;
use raft_kv_memstore_grpc::protobuf::app_service_client::AppServiceClient;
use tokio::runtime::Runtime;
use tonic::transport::Channel;
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

/// Set up a cluster of 3 nodes.
/// Write to it and read from it.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn test_cluster() -> anyhow::Result<()> {
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

    // --- Start 3 raft node in 3 threads.

    let _h1 = thread::spawn(|| {
        let rt = Runtime::new().unwrap();
        let x = rt.block_on(start_raft_app(1, get_addr(1)));
        println!("raft app exit result: {:?}", x);
    });

    let _h2 = thread::spawn(|| {
        let rt = Runtime::new().unwrap();
        let x = rt.block_on(start_raft_app(2, get_addr(2)));
        println!("raft app exit result: {:?}", x);
    });

    let _h3 = thread::spawn(|| {
        let rt = Runtime::new().unwrap();
        let x = rt.block_on(start_raft_app(3, get_addr(3)));
        println!("raft app exit result: {:?}", x);
    });

    // Wait for server to start up.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let mut client1 = new_client(get_addr(1)).await?;

    // --- Initialize the target node as a cluster of only one node.
    //     After init(), the single node cluster will be fully functional.
    println!("=== init single node cluster");
    {
        client1
            .init(pb::InitRequest {
                nodes: vec![new_node(1)],
            })
            .await?;

        let metrics = client1.metrics(()).await?.into_inner();
        println!("=== metrics after init: {:?}", metrics);
    }

    println!(
        "=== Add node 2, 3 to the cluster as learners, to let them start to receive log replication from the leader"
    );
    {
        println!("=== add-learner 2");
        client1
            .add_learner(pb::AddLearnerRequest {
                node: Some(new_node(2)),
            })
            .await?;

        println!("=== add-learner 3");
        client1
            .add_learner(pb::AddLearnerRequest {
                node: Some(new_node(3)),
            })
            .await?;

        let metrics = client1.metrics(()).await?.into_inner();
        println!("=== metrics after add-learner: {:?}", metrics);
        assert_eq!(
            vec![pb::NodeIdSet {
                node_ids: btreemap! { 1 => ()}
            }],
            metrics.membership.clone().unwrap().configs
        );
        assert_eq!(
            btreemap! {
                1=>new_node(1),
                2=>new_node(2),
                3=>new_node(3),
            },
            metrics.membership.unwrap().nodes
        );
    }

    // --- Turn the two learners to members.
    //     A member node can vote or elect itself as leader.

    println!("=== change-membership to 1,2,3");
    {
        client1
            .change_membership(pb::ChangeMembershipRequest {
                members: vec![1, 2, 3],
                retain: false,
            })
            .await?;

        let metrics = client1.metrics(()).await?.into_inner();
        println!("=== metrics after change-member: {:?}", metrics);
        assert_eq!(
            vec![pb::NodeIdSet {
                node_ids: btreemap! { 1=>(),2=>(),3=>()}
            }],
            metrics.membership.unwrap().configs
        );
    }

    println!("=== write `foo=bar`");
    {
        client1
            .set(pb::SetRequest {
                key: "foo".to_string(),
                value: "bar".to_string(),
            })
            .await?;

        // --- Wait for a while to let the replication get done.
        tokio::time::sleep(Duration::from_millis(1_000)).await;
    }

    println!("=== read `foo` on every node");
    {
        println!("=== read `foo` on node 1");
        {
            let got = client1.get(pb::GetRequest { key: "foo".to_string() }).await?;
            assert_eq!(Some("bar".to_string()), got.into_inner().value);
        }

        println!("=== read `foo` on node 2");
        {
            let mut client2 = new_client(get_addr(2)).await?;
            let got = client2.get(pb::GetRequest { key: "foo".to_string() }).await?;
            assert_eq!(Some("bar".to_string()), got.into_inner().value);
        }

        println!("=== read `foo` on node 3");
        {
            let mut client3 = new_client(get_addr(3)).await?;
            let got = client3.get(pb::GetRequest { key: "foo".to_string() }).await?;
            assert_eq!(Some("bar".to_string()), got.into_inner().value);
        }
    }

    println!("=== Remove node 1,2 by change-membership to {{3}}");
    {
        client1
            .change_membership(pb::ChangeMembershipRequest {
                members: vec![3],
                retain: false,
            })
            .await?;

        tokio::time::sleep(Duration::from_millis(2_000)).await;

        let metrics = client1.metrics(()).await?.into_inner();
        println!("=== metrics after change-membership to {{3}}: {:?}", metrics);
        assert_eq!(
            vec![pb::NodeIdSet {
                node_ids: btreemap! { 3=>() }
            }],
            metrics.membership.unwrap().configs
        );
    }

    Ok(())
}

async fn new_client(addr: String) -> Result<AppServiceClient<Channel>, tonic::transport::Error> {
    let channel = Channel::builder(format!("https://{}", addr).parse().unwrap()).connect().await?;
    let client = AppServiceClient::new(channel);
    Ok(client)
}

fn new_node(node_id: u64) -> pb::Node {
    pb::Node {
        node_id,
        rpc_addr: get_addr(node_id),
    }
}

fn get_addr(node_id: u64) -> String {
    match node_id {
        1 => "127.0.0.1:21001".to_string(),
        2 => "127.0.0.1:21002".to_string(),
        3 => "127.0.0.1:21003".to_string(),
        _ => {
            unreachable!("node_id must be 1, 2, or 3");
        }
    }
}
