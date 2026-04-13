#![allow(clippy::uninlined_format_args)]
use std::backtrace::Backtrace;
use std::panic::PanicHookInfo;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use maplit::btreemap;
use openraft::async_runtime::AsyncRuntime;
use openraft::type_config::TypeConfigExt;
use openraft::type_config::alias::AsyncRuntimeOf;
use raft_kv_memstore_grpc::TypeConfig;
use raft_kv_memstore_grpc::app::start_raft_app;
use raft_kv_memstore_grpc::grpc::app_service::LEADER_ENDPOINT_HEADER;
use raft_kv_memstore_grpc::protobuf as pb;
use raft_kv_memstore_grpc::protobuf::app_service_client::AppServiceClient;
use tonic::Code;
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

// --- Forwarding gRPC Client ---

/// A gRPC client that automatically forwards requests to the Raft leader.
///
/// When a non-leader node returns `Status::Unavailable` with the
/// `x-openraft-leader-endpoint` metadata header, this client extracts
/// the leader address, reconnects, and retries.
struct GrpcClient {
    /// Current leader address, shared so it can be updated on forwarding.
    leader: Arc<Mutex<String>>,
    inner: AppServiceClient<Channel>,
}

impl GrpcClient {
    async fn new(addr: String) -> anyhow::Result<Self> {
        let inner = connect(&addr).await?;
        Ok(Self {
            leader: Arc::new(Mutex::new(addr)),
            inner,
        })
    }

    /// Send a `Set` request, retrying up to 3 times on leader forwarding.
    async fn set(&mut self, req: pb::SetRequest) -> anyhow::Result<pb::Response> {
        self.with_forwarding(|c| {
            let r = req.clone();
            Box::pin(async move { c.set(r).await })
        })
        .await
    }

    /// Send an `AddLearner` request with forwarding.
    async fn add_learner(&mut self, req: pb::AddLearnerRequest) -> anyhow::Result<pb::ClientWriteResponse> {
        self.with_forwarding(|c| {
            let r = req.clone();
            Box::pin(async move { c.add_learner(r).await })
        })
        .await
    }

    /// Send a `ChangeMembership` request with forwarding.
    async fn change_membership(&mut self, req: pb::ChangeMembershipRequest) -> anyhow::Result<pb::ClientWriteResponse> {
        self.with_forwarding(|c| {
            let r = req.clone();
            Box::pin(async move { c.change_membership(r).await })
        })
        .await
    }

    /// Generic retry loop: on `Unavailable` with leader metadata, switch endpoint and retry.
    async fn with_forwarding<T, F>(&mut self, make_rpc: F) -> anyhow::Result<T>
    where F: Fn(
            &mut AppServiceClient<Channel>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<tonic::Response<T>, tonic::Status>> + Send + '_>,
        > {
        let max_retries = 3;

        for _attempt in 0..=max_retries {
            let result = make_rpc(&mut self.inner).await;

            match result {
                Ok(resp) => return Ok(resp.into_inner()),
                Err(status) if status.code() == Code::Unavailable => {
                    // Extract leader endpoint from gRPC metadata
                    let leader_addr = status
                        .metadata()
                        .get(LEADER_ENDPOINT_HEADER)
                        .and_then(|v| v.to_str().ok())
                        .map(|s| s.to_string());

                    if let Some(addr) = leader_addr {
                        println!(">>> forwarding to leader at {}", addr);
                        *self.leader.lock().unwrap() = addr.clone();
                        self.inner = connect(&addr).await?;
                        continue;
                    }

                    return Err(anyhow::anyhow!(
                        "Unavailable but no leader endpoint in metadata: {}",
                        status
                    ));
                }
                Err(status) => {
                    return Err(anyhow::anyhow!("RPC failed: {}", status));
                }
            }
        }

        Err(anyhow::anyhow!("max retries exceeded"))
    }
}

async fn connect(addr: &str) -> Result<AppServiceClient<Channel>, tonic::transport::Error> {
    let channel = Channel::builder(format!("https://{}", addr).parse().unwrap()).connect().await?;
    Ok(AppServiceClient::new(channel))
}

// --- Test ---

/// Set up a cluster of 3 nodes.
/// Write to it and read from it, including writes to non-leader nodes
/// to demonstrate automatic leader forwarding.
#[test]
fn test_cluster() {
    TypeConfig::run(test_cluster_inner()).unwrap();
}

async fn test_cluster_inner() -> anyhow::Result<()> {
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
        let mut rt = AsyncRuntimeOf::<TypeConfig>::new(1);
        let x = rt.block_on(start_raft_app(1, get_addr(1)));
        println!("raft app exit result: {:?}", x);
    });

    let _h2 = thread::spawn(|| {
        let mut rt = AsyncRuntimeOf::<TypeConfig>::new(1);
        let x = rt.block_on(start_raft_app(2, get_addr(2)));
        println!("raft app exit result: {:?}", x);
    });

    let _h3 = thread::spawn(|| {
        let mut rt = AsyncRuntimeOf::<TypeConfig>::new(1);
        let x = rt.block_on(start_raft_app(3, get_addr(3)));
        println!("raft app exit result: {:?}", x);
    });

    // Wait for server to start up.
    TypeConfig::sleep(Duration::from_millis(200)).await;

    // Use the forwarding client that automatically retries on leader redirection.
    let mut client1 = GrpcClient::new(get_addr(1)).await?;

    // --- Initialize the target node as a cluster of only one node.
    //     After init(), the single node cluster will be fully functional.
    println!("=== init single node cluster");
    {
        // init() does not go through the forwarding client because it's only
        // called on the node that will become the initial leader.
        client1
            .inner
            .init(pb::InitRequest {
                nodes: vec![new_node(1)],
            })
            .await?;

        let metrics = client1.inner.metrics(()).await?.into_inner();
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

        let metrics = client1.inner.metrics(()).await?.into_inner();
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

        let metrics = client1.inner.metrics(()).await?.into_inner();
        println!("=== metrics after change-member: {:?}", metrics);
        assert_eq!(
            vec![pb::NodeIdSet {
                node_ids: btreemap! { 1=>(),2=>(),3=>()}
            }],
            metrics.membership.unwrap().configs
        );
    }

    // --- Write via the leader (node 1).

    println!("=== write `foo=bar` on leader (node 1)");
    {
        client1
            .set(pb::SetRequest {
                key: "foo".to_string(),
                value: "bar".to_string(),
            })
            .await?;

        TypeConfig::sleep(Duration::from_millis(500)).await;
    }

    // --- Write via a non-leader node (node 2).
    //     This demonstrates leader forwarding: node 2 returns `Unavailable`
    //     with the leader's endpoint in metadata, and the client retries
    //     against the leader automatically.

    println!("=== write `qux=quux` on non-leader (node 2), expect auto-forwarding to leader");
    {
        let mut client2 = GrpcClient::new(get_addr(2)).await?;
        client2
            .set(pb::SetRequest {
                key: "qux".to_string(),
                value: "quux".to_string(),
            })
            .await?;

        TypeConfig::sleep(Duration::from_millis(500)).await;
    }

    // --- Read from every node to verify replication.

    println!("=== read `foo` and `qux` on every node");
    {
        for node_id in [1, 2, 3] {
            println!("=== read on node {}", node_id);
            let mut client = connect(&get_addr(node_id)).await?;

            let got_foo = client.get(pb::GetRequest { key: "foo".to_string() }).await?;
            assert_eq!(Some("bar".to_string()), got_foo.into_inner().value);

            let got_qux = client.get(pb::GetRequest { key: "qux".to_string() }).await?;
            assert_eq!(Some("quux".to_string()), got_qux.into_inner().value);
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

        TypeConfig::sleep(Duration::from_millis(2_000)).await;

        let metrics = client1.inner.metrics(()).await?.into_inner();
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
