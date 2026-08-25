use std::io;
use std::time::Duration;

use maplit::btreemap;
use openraft::NodeInfo;
use openraft::ServerState;
use openraft::type_config::TypeConfigExt;
use raft_kv_log_wal_sm_mem::Raft;
use raft_kv_log_wal_sm_mem::StateMachineStore;
use raft_kv_log_wal_sm_mem::TypeConfig;
use raft_kv_log_wal_sm_mem::example_config;
use raft_kv_log_wal_sm_mem::new_raft_node;
use tempfile::TempDir;

/// A restarted node rebuilds its state machine from the WAL.
///
/// `sm-mem` loses its data on restart and `example_config()` builds no
/// snapshot, so re-applying committed entries the WAL still holds is the only
/// way the second node can serve a key the first one wrote.
#[test]
fn restart_rebuilds_state_machine_from_wal() {
    TypeConfig::run(restart_inner()).unwrap();
}

async fn restart_inner() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let root = TempDir::new()?;
    // The WAL directory itself does not exist yet, matching what `--data-dir`
    // names on a first start.
    let data_dir = root.path().join("n1.wal").display().to_string();

    let written = {
        tracing::info!("start node 1 as a single-node cluster and write foo=bar");

        let (raft, _state_machine) = new_raft_node(1, data_dir.clone(), example_config()).await?;

        let node = NodeInfo::new("127.0.0.1:34001", "127.0.0.1:33001");
        raft.initialize(btreemap! {1 => node}).await?;
        raft.wait(Some(Duration::from_secs(5))).state(ServerState::Leader, "node 1 leads").await?;

        let resp = raft.client_write(types_kv::Request::set("foo", "bar")).await?;

        // Shutting down joins the Raft core task, which drops the log store.
        // Dropping it flushes the WAL and releases the directory lock that the
        // next open needs.
        raft.shutdown().await?;

        resp.data
    };

    tracing::info!("reopen the same WAL directory with a fresh state machine");
    {
        let (raft, state_machine) = reopen(&data_dir).await?;

        let value = state_machine.get("foo").await;
        assert_eq!(written.value, value);

        raft.shutdown().await?;
    }

    Ok(())
}

/// Open the node in `data_dir`, waiting for the previous one to let go of it.
///
/// `Raft::shutdown()` joins the Raft core task, but the state-machine worker
/// that core spawned holds a log reader of its own and drops it a moment later.
/// `raft-log` releases the directory lock with that last clone, so an immediate
/// reopen loses a race that a retry wins. An application restarting a node in
/// the same process needs the same retry.
async fn reopen(data_dir: &str) -> Result<(Raft, StateMachineStore), Box<dyn std::error::Error + Send + Sync>> {
    let deadline = 50;

    for _ in 0..deadline {
        let res = new_raft_node(1, data_dir.to_string(), example_config()).await;

        match res {
            Ok(node) => return Ok(node),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                TypeConfig::sleep(Duration::from_millis(100)).await;
            }
            Err(e) => return Err(e.into()),
        }
    }

    Err(format!("{data_dir} is still locked after {deadline} attempts").into())
}
