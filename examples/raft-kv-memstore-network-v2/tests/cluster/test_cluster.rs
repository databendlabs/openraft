use std::backtrace::Backtrace;
use std::panic::PanicHookInfo;
use std::time::Duration;

use app_http::AddLearnerRequest;
use app_http::Client;
use openraft::ServerState;
use openraft::type_config::TypeConfigExt;
use raft_kv_memstore_network_v2::Raft;
use raft_kv_memstore_network_v2::TypeConfig;
use raft_kv_memstore_network_v2::new_raft_node;
use raft_kv_memstore_network_v2::run_raft_node;
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

/// This test shows how to transfer a snapshot from one node to another:
///
/// - Setup a single node cluster, write some logs, take a snapshot;
/// - Add a learner node-2 to receive snapshot replication, via the complete-snapshot API:
///   - The sending end sends snapshot with `RaftNetwork::full_snapshot()`;
///   - The receiving end deliver the received snapshot to `Raft` with
///     `Raft::install_full_snapshot()`.
#[test]
fn test_cluster() {
    TypeConfig::run(async {
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

        let app1 = new_raft_node(1, api_addr(1), raft_addr(1)).await;
        let app2 = new_raft_node(2, api_addr(2), raft_addr(2)).await;

        let rafts = [app1.raft.clone(), app2.raft.clone()];

        for app in [app1, app2] {
            TypeConfig::spawn(run_raft_node(app));
        }

        run_test(&rafts).await;
    });
}

fn api_addr(node_id: u64) -> String {
    format!("127.0.0.1:21{:03}", node_id)
}

fn raft_addr(node_id: u64) -> String {
    format!("127.0.0.1:23{:03}", node_id)
}

async fn run_test(rafts: &[Raft]) {
    // Wait for server to start up.
    TypeConfig::sleep(Duration::from_millis(200)).await;

    let raft1 = &rafts[0];
    let raft2 = &rafts[1];
    let client = Client::<TypeConfig>::new(1, api_addr(1));

    println!("=== init single node cluster");
    {
        client.init().await.unwrap().unwrap();
        raft1.wait(None).state(ServerState::Leader, "wait node 1 to become leader").await.unwrap();
    }

    println!("=== write 2 logs");
    {
        let resp = client.write(&types_kv::Request::set("foo1", "bar1")).await.unwrap().unwrap();
        println!("write resp: {:#?}", resp);
        let resp = client.write(&types_kv::Request::set("foo2", "bar2")).await.unwrap().unwrap();
        println!("write resp: {:#?}", resp);
    }

    println!("=== let node-1 take a snapshot");
    {
        raft1.trigger().snapshot().await.unwrap();

        // Wait for a while to let the snapshot get done.
        TypeConfig::sleep(Duration::from_millis(500)).await;
    }

    println!("=== metrics after building snapshot");
    {
        let metrics = client.metrics().await.unwrap();
        println!("node 1 metrics: {:#?}", metrics);
        assert_eq!(Some(3), metrics.snapshot.map(|x| x.index));
        assert_eq!(Some(3), metrics.purged.map(|x| x.index));
    }

    println!("=== add-learner node-2");
    {
        let resp = client
            .add_learner(&AddLearnerRequest {
                node_id: 2,
                api_addr: api_addr(2),
                raft_addr: raft_addr(2),
            })
            .await
            .unwrap()
            .unwrap();
        println!("add-learner node-2 resp: {:#?}", resp);
    }

    // Wait for a while to let the node 2 to receive snapshot replication.
    TypeConfig::sleep(Duration::from_millis(500)).await;

    println!("=== metrics of node 2 that received snapshot");
    {
        let client2 = Client::<TypeConfig>::new(2, api_addr(2));
        let metrics = client2.metrics().await.unwrap();
        println!("node 2 metrics: {:#?}", metrics);
        assert_eq!(Some(3), metrics.snapshot.map(|x| x.index));
        assert_eq!(Some(3), metrics.purged.map(|x| x.index));
    }

    // In this example, the snapshot is just a copy of the state machine.
    let snapshot = raft2.get_snapshot().await.unwrap();
    println!("node 2 received snapshot: {:#?}", snapshot);
}
