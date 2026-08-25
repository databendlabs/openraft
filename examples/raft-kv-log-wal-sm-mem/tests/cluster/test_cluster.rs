use std::thread;
use std::time::Duration;

use app_http::AddLearnerRequest;
use app_http::Client;
use maplit::btreeset;
use openraft::async_runtime::AsyncRuntime;
use openraft::type_config::TypeConfigExt;
use openraft::type_config::alias::AsyncRuntimeOf;
use raft_kv_log_wal_sm_mem::TypeConfig;
use raft_kv_log_wal_sm_mem::start_example_raft_node;
use tempfile::TempDir;
use tracing_subscriber::EnvFilter;

/// Node `n` serves the app API on `33000 + n` and Raft on `34000 + n`. No other
/// example test binds this range.
const API_PORT_BASE: u16 = 33000;
const RAFT_PORT_BASE: u16 = 34000;

fn api_addr(node_id: u64) -> String {
    format!("127.0.0.1:{}", API_PORT_BASE + node_id as u16)
}

fn raft_addr(node_id: u64) -> String {
    format!("127.0.0.1:{}", RAFT_PORT_BASE + node_id as u16)
}

/// Form a 3-node cluster over HTTP, write one key, and read it back on every
/// node.
///
/// This is the only test that starts the example the way its README does, so it
/// is also what covers opening a WAL directory that does not exist yet.
#[test]
fn test_cluster() {
    TypeConfig::run(test_cluster_inner()).unwrap();
}

async fn test_cluster_inner() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _ = tracing_subscriber::fmt()
        .with_target(true)
        .with_thread_ids(true)
        .with_level(true)
        .with_ansi(false)
        .with_env_filter(EnvFilter::from_default_env())
        .try_init();

    tracing::info!("start 3 nodes, each on its own thread, runtime and WAL directory");
    {
        for node_id in 1..=3u64 {
            let temp_dir = TempDir::new()?;
            // The WAL directory itself does not exist yet, matching what
            // `--data-dir` names on a first start.
            let data_dir = temp_dir.path().join("wal").display().to_string();

            thread::spawn(move || {
                // The node never returns, so moving `temp_dir` in keeps the
                // directory alive for as long as the node runs.
                let _temp_dir = temp_dir;

                let mut rt = AsyncRuntimeOf::<TypeConfig>::new(1);
                let res = rt.block_on(start_example_raft_node(
                    node_id,
                    data_dir,
                    api_addr(node_id),
                    raft_addr(node_id),
                ));
                tracing::info!("node {} exited: {:?}", node_id, res);
            });
        }

        TypeConfig::sleep(Duration::from_millis(3_000)).await;
    }

    let leader = Client::<TypeConfig>::new(1, api_addr(1));

    tracing::info!("initialize node 1 as a single-node cluster and wait until it leads");
    {
        leader.init().await??;

        loop {
            let metrics = leader.metrics().await?;
            if metrics.current_leader == Some(1) {
                break;
            }
            TypeConfig::sleep(Duration::from_millis(200)).await;
        }
    }

    tracing::info!("add node 2 and 3 as learners, then turn all three into voters");
    {
        for node_id in [2, 3] {
            leader
                .add_learner(&AddLearnerRequest {
                    node_id,
                    api_addr: api_addr(node_id),
                    raft_addr: raft_addr(node_id),
                })
                .await??;
        }

        leader.change_membership(&btreeset! {1, 2, 3}).await??;

        let metrics = leader.metrics().await?;
        let joint_config = metrics.membership_config.membership().get_joint_config();
        assert_eq!(&vec![btreeset! {1, 2, 3}], joint_config);
    }

    let written = {
        tracing::info!("write foo=bar on the leader");

        let resp = leader.write(&types_kv::Request::set("foo", "bar")).await??;
        resp.data
    };

    tracing::info!("read foo=bar back on every node, the two followers included");
    {
        TypeConfig::sleep(Duration::from_millis(500)).await;

        for node_id in 1..=3u64 {
            let client = Client::<TypeConfig>::new(node_id, api_addr(node_id));
            let got = client.read(&"foo".to_string()).await?;
            assert_eq!(written, got, "node {}", node_id);
        }
    }

    Ok(())
}
