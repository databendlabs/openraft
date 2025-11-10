#![allow(clippy::uninlined_format_args)]
use std::backtrace::Backtrace;
use std::panic::PanicHookInfo;
use std::thread;
use std::time::Duration;

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

/// Test that append_entries RPC automatically chunks when payload is too large.
/// This test verifies that log entries are correctly replicated even when
/// the gRPC message size limit is exceeded, by automatically splitting into chunks.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_chunk_append_entries() -> anyhow::Result<()> {
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

    tokio::time::sleep(Duration::from_millis(200)).await;

    let mut client1 = new_client(get_addr(1)).await?;

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

    println!("=== Add node 2 as learner");
    {
        client1
            .add_learner(pb::AddLearnerRequest {
                node: Some(new_node(2)),
            })
            .await?;

        let metrics = client1.metrics(()).await?.into_inner();
        println!("=== metrics after add-learner: {:?}", metrics);
    }

    println!("=== Write multiple entries to trigger chunking");
    {
        // Write 10 entries with large values (200 bytes each).
        // With CHUNK_SIZE=2 and 1KB message limit, this should trigger
        // multiple chunked append_entries RPCs during replication.
        let large_value = "x".repeat(200);
        for i in 0..10 {
            client1
                .set(pb::SetRequest {
                    key: format!("key_{}", i),
                    value: format!("{}_{}", large_value, i),
                })
                .await?;
        }

        // Wait for replication to complete
        tokio::time::sleep(Duration::from_millis(2_000)).await;
    }

    println!("=== Verify all entries are replicated to node 2");
    {
        let mut client2 = new_client(get_addr(2)).await?;
        let large_value = "x".repeat(200);

        for i in 0..10 {
            let got = client2
                .get(pb::GetRequest {
                    key: format!("key_{}", i),
                })
                .await?;
            assert_eq!(
                Some(format!("{}_{}", large_value, i)),
                got.into_inner().value,
                "key_{} should have correct value on node 2",
                i
            );
        }

        println!("=== All entries successfully replicated via chunking");
    }

    Ok(())
}

async fn new_client(addr: String) -> Result<AppServiceClient<Channel>, tonic::transport::Error> {
    let channel = Channel::builder(format!("http://{}", addr).parse().unwrap()).connect().await?;
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
        1 => "127.0.0.1:22001".to_string(),
        2 => "127.0.0.1:22002".to_string(),
        _ => {
            unreachable!("node_id must be 1 or 2");
        }
    }
}
