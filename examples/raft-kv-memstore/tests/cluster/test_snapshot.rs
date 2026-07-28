use std::time::Duration;

use app_http::AddLearnerRequest;
use app_http::Client;
use openraft::Config;
use openraft::SnapshotPolicy;
use openraft::type_config::TypeConfigExt;
use raft_kv_memstore::TypeConfig;
use raft_kv_memstore::example_config;

use crate::util;

/// Distinct port range so this test never collides with the others run in parallel.
const PORT_BASE: u16 = 27000;

/// Bring a learner up to date by transferring a snapshot instead of log entries.
///
/// The leader snapshots after every committed log and keeps none of the logs the
/// snapshot covers, so by the time node 2 joins there are no entries left to
/// replicate and the only way to catch it up is the full-snapshot API:
///
/// - the sending end streams it with `RaftNetworkV2::full_snapshot()`;
/// - the receiving end hands it to `Raft::install_full_snapshot()`.
#[test]
fn test_snapshot_replication() {
    TypeConfig::run(test_snapshot_replication_inner()).unwrap();
}

async fn test_snapshot_replication_inner() -> anyhow::Result<()> {
    let config = Config {
        snapshot_policy: SnapshotPolicy::LogsSinceLast(1),
        max_in_snapshot_log_to_keep: 0,
        ..example_config()
    };

    util::spawn_nodes_with_config(PORT_BASE, 2, config);
    TypeConfig::sleep(Duration::from_millis(1_000)).await;

    let leader = util::init_leader(PORT_BASE).await?;

    println!("=== write 2 logs on the leader");
    leader.write(&types_kv::Request::set("foo1", "bar1")).await??;
    let expected_foo2 = leader.write(&types_kv::Request::set("foo2", "bar2")).await??.data;

    println!("=== leader snapshots and purges the logs it covers");
    // Building the snapshot and purging the logs it covers are two steps, so wait for
    // `purged` to catch up with `snapshot` instead of asserting they move together.
    let leader_snapshot = loop {
        let metrics = leader.metrics().await?;
        if let Some(snapshot) = metrics.snapshot
            && metrics.purged.map(|x| x.index()) == Some(snapshot.index())
        {
            break snapshot;
        }
        TypeConfig::sleep(Duration::from_millis(200)).await;
    };

    println!("=== add-learner node-2, which can only catch up from the snapshot");
    leader
        .add_learner(&AddLearnerRequest {
            node_id: 2,
            api_addr: util::api_addr(PORT_BASE, 2),
            raft_addr: util::raft_addr(PORT_BASE, 2),
        })
        .await??;

    println!("=== node-2 installs the received snapshot");
    // Adding the learner appends a membership log, which the leader immediately snapshots
    // too, so node-2 may well install that later snapshot rather than the one observed
    // above. Any snapshot reaching that index proves node-2 caught up without the logs.
    let learner = Client::<TypeConfig>::new(2, util::api_addr(PORT_BASE, 2));
    loop {
        let metrics = learner.metrics().await?;
        if metrics.snapshot.map(|x| x.index()) >= Some(leader_snapshot.index()) {
            break;
        }
        TypeConfig::sleep(Duration::from_millis(200)).await;
    }

    println!("=== node-2 serves the state carried by the snapshot");
    assert_eq!(expected_foo2, learner.read(&"foo2".to_string()).await?);

    Ok(())
}
