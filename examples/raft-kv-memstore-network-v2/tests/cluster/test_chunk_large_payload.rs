use std::backtrace::Backtrace;
use std::collections::BTreeMap;
use std::panic::PanicHookInfo;
use std::sync::Arc;
use std::time::Duration;

use openraft::BasicNode;
use openraft::Config;
use raft_kv_memstore_network_v2::router::Router;
use raft_kv_memstore_network_v2::store::Request;
use raft_kv_memstore_network_v2::typ;
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

/// Test that demonstrates automatic payload chunking when transport size limits are hit.
///
/// This test shows how the network layer handles large payloads that exceed transport limits:
/// - Configure a small `max_payload_entries` to create large batches
/// - Set a strict transport size limit on the router
/// - Write multiple entries that would exceed the limit in a single request
/// - Verify that the network layer automatically chunks and retries
/// - Confirm all entries are successfully replicated
#[tokio::test]
async fn test_chunk_large_payload() {
    std::panic::set_hook(Box::new(|panic| {
        log_panic(panic);
    }));

    let _ = tracing_subscriber::fmt()
        .with_target(true)
        .with_thread_ids(true)
        .with_level(true)
        .with_ansi(false)
        .with_env_filter(EnvFilter::from_default_env())
        .try_init();

    let router = Router::default();

    let local = LocalSet::new();

    // Create rafts with custom config
    let config = Arc::new(
        Config {
            heartbeat_interval: 500,
            election_timeout_min: 1500,
            election_timeout_max: 3000,
            max_payload_entries: 3,
            ..Default::default()
        }
        .validate()
        .unwrap(),
    );

    let (raft1, app1) = new_raft_with_config(1, router.clone(), config.clone()).await;
    let (raft2, app2) = new_raft_with_config(2, router.clone(), config.clone()).await;

    let rafts = [raft1, raft2];

    local
        .run_until(async move {
            task::spawn_local(app1.run());
            task::spawn_local(app2.run());

            run_test(&rafts, router).await;
        })
        .await;
}

async fn new_raft_with_config(
    node_id: u64,
    router: Router,
    config: Arc<Config>,
) -> (typ::Raft, raft_kv_memstore_network_v2::app::App) {
    use raft_kv_memstore_network_v2::store::LogStore;
    use raft_kv_memstore_network_v2::store::StateMachineStore;

    let log_store = LogStore::default();
    let state_machine_store = Arc::new(StateMachineStore::default());

    let raft = openraft::Raft::new(node_id, config, router.clone(), log_store, state_machine_store.clone())
        .await
        .unwrap();

    let app = raft_kv_memstore_network_v2::app::App::new(node_id, raft.clone(), router, state_machine_store);

    (raft, app)
}

async fn run_test(rafts: &[typ::Raft], router: Router) {
    // Wait for server to start up.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let raft1 = &rafts[0];
    let raft2 = &rafts[1];

    println!("=== init two-node cluster");
    {
        let mut nodes = BTreeMap::new();
        nodes.insert(1, BasicNode { addr: "".to_string() });
        nodes.insert(2, BasicNode { addr: "".to_string() });
        raft1.initialize(nodes).await.unwrap();
    }

    println!("=== wait for initial log replication");
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Set a very small payload size limit to force chunking
    // This simulates a transport layer that rejects large payloads (e.g., HTTP 413)
    // Based on testing, a single entry is ~250-300 bytes, 2 entries is ~500-600 bytes
    println!("=== set payload size limit to 450 bytes to force chunking when multiple entries");
    router.set_max_payload_size(Some(450));

    println!("=== write 10 entries rapidly to create larger batches");
    {
        // Write entries quickly so they batch together in replication
        let mut handles = vec![];
        for i in 0..10 {
            let raft = raft1.clone();
            let handle = tokio::spawn(async move {
                let key = format!("key{}", i);
                let value = "x".repeat(50); // Smaller individual entries
                raft.client_write(Request::set(&key, &value)).await
            });
            handles.push(handle);
        }

        for (i, handle) in handles.into_iter().enumerate() {
            let resp = handle.await.unwrap();
            println!("write {} resp: {:?}", i, resp.as_ref().map(|r| &r.log_id));
            assert!(resp.is_ok(), "write should succeed even with payload limits");
        }
    }

    println!("=== wait for replication to complete");
    tokio::time::sleep(Duration::from_millis(2000)).await;

    println!("=== verify all entries replicated to follower");
    {
        let metrics1 = raft1.metrics().borrow().clone();
        let metrics2 = raft2.metrics().borrow().clone();

        println!("raft1 last_log_index: {:?}", metrics1.last_log_index);
        println!("raft2 last_log_index: {:?}", metrics2.last_log_index);
        println!("raft1 last_applied: {:?}", metrics1.last_applied);
        println!("raft2 last_applied: {:?}", metrics2.last_applied);

        assert_eq!(
            metrics1.last_log_index, metrics2.last_log_index,
            "Both nodes should have the same last log index"
        );
        assert_eq!(
            metrics1.last_applied, metrics2.last_applied,
            "Both nodes should have applied the same logs"
        );
    }

    println!("=== test passed: payload chunking works correctly");
}
