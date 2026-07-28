//! Targeted deterministic scenario tests.
//!
//! Each test pins the exact event sequence that arms one historical-bug trap
//! which the randomized fuzzer is unlikely to compose on its own. The
//! simulation runs under a fixed seed, so a failure is always reproducible.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use openraft::async_runtime::WatchReceiver;
use rand::SeedableRng;
use rand::rngs::SmallRng;

use crate::cluster::ClusterState;
use crate::cluster::host_name;
use crate::cluster::register_node_storage;
use crate::cluster::spawn_host;
use crate::liveness;
use crate::typ::*;

/// Poll `f` until it returns `Some`, panicking after a virtual-time deadline.
async fn wait_for<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let deadline = turmoil::elapsed() + Duration::from_secs(60);
    loop {
        if let Some(v) = f() {
            return v;
        }
        if turmoil::elapsed() > deadline {
            panic!("scenario timeout waiting for: {what}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Regression trap for openraft [#1828] (fixed in afd6508b): a learner added
/// while unreachable, followed by a full log purge on the leader with zero
/// appends in between, leaves the learner's progress entry probing with
/// `searching_end == purge_upto_next == last_next`. The probe range is then
/// empty (`start == end`) and the only way forward is a snapshot; the buggy
/// code produced `Inflight::None` instead, and replication stalled forever —
/// even after the network healed.
///
/// Sequence: voters {1, 2} plus node 3 → partition 3 → `add_learner(3)` →
/// no writes → snapshot + purge-to-tip on the leader → repair 3 → require
/// full convergence (node 3 can only catch up via snapshot: the leader's
/// log is empty).
///
/// [#1828]: https://github.com/databendlabs/openraft/issues/1828
#[test]
fn learner_added_while_partitioned_catches_up_from_fully_purged_leader() {
    const SEED: u64 = 0x1828;

    // Reset the `futures_util::select!` shuffle RNG (a process-wide
    // thread-local) so this test is deterministic no matter what ran on
    // this thread before it.
    futures_util::reseed(SEED);

    let mut sim = turmoil::Builder::new()
        .simulation_duration(Duration::from_secs(600))
        .tcp_capacity(65536)
        .build_with_rng(Box::new(SmallRng::seed_from_u64(SEED)));

    let raft_config = Arc::new(openraft::Config {
        heartbeat_interval: 50,
        election_timeout_min: 150,
        election_timeout_max: 300,
        // The test controls compaction: no automatic snapshot before the
        // explicit trigger, and no log held back from the explicit purge.
        snapshot_policy: openraft::SnapshotPolicy::LogsSinceLast(1_000_000),
        max_in_snapshot_log_to_keep: 0,
        ..Default::default()
    });

    let cluster = Arc::new(Mutex::new(ClusterState::new()));

    let all_nodes: BTreeMap<NodeId, Node> = (1..=3)
        .map(|id| {
            (id, Node {
                addr: format!("{}:9000", host_name(id)),
            })
        })
        .collect();
    // Only {1, 2} bootstrap the cluster; node 3 comes up uninitialized.
    let initial_nodes: BTreeMap<NodeId, Node> =
        all_nodes.iter().filter(|(id, _)| **id <= 2).map(|(id, n)| (*id, n.clone())).collect();

    for id in 1..=3 {
        register_node_storage(id, &cluster);
        spawn_host(
            &mut sim,
            id,
            raft_config.clone(),
            cluster.clone(),
            SEED,
            initial_nodes.clone(),
        );
    }

    let cs = cluster.clone();
    let learner = all_nodes[&3].clone();
    sim.client("scenario", async move {
        let (_, raft) = wait_for("a leader among the initial voters", || {
            cs.lock().unwrap().find_leader_entry()
        })
        .await;

        // A membership change is rejected until the leader has committed its
        // initial entries; wait until it is fully applied.
        wait_for("leader fully applied", || {
            let m = raft.metrics().borrow_watched().clone();
            let applied = m.last_applied.map(|l| l.index);
            (applied.is_some() && applied == m.last_log_index).then_some(())
        })
        .await;

        // Node 3 must see nothing until the leader's log is purged.
        turmoil::partition(host_name(3), host_name(1));
        turmoil::partition(host_name(3), host_name(2));

        // One membership entry; the leader creates node 3's progress entry
        // with `searching_end == last_next`. Non-blocking: replication to 3
        // is cut, only the {1, 2} commit matters.
        let resp = raft.add_learner(3, learner, false).await?;
        let last_log = resp.log_id.index;

        // The snapshot is built at `last_applied`; wait until the membership
        // entry is applied so the purge below can cover the whole log.
        wait_for("membership entry applied", || {
            let m = raft.metrics().borrow_watched().clone();
            (m.last_applied.map(|l| l.index) == Some(last_log) && m.last_log_index == Some(last_log)).then_some(())
        })
        .await;

        // Compact to the tip: afterwards `purge_upto_next == last_next ==
        // searching_end` — the #1828 equality corner.
        raft.trigger().snapshot().await?;
        wait_for("snapshot at the log tip", || {
            (raft.metrics().borrow_watched().snapshot.as_ref().map(|s| s.index) == Some(last_log)).then_some(())
        })
        .await;
        raft.trigger().purge_log(last_log).await?;
        wait_for("log fully purged", || {
            (raft.metrics().borrow_watched().purged.as_ref().map(|p| p.index) == Some(last_log)).then_some(())
        })
        .await;

        // Idle window: nothing may repopulate the log before the heal.
        tokio::time::sleep(Duration::from_millis(500)).await;

        turmoil::repair(host_name(3), host_name(1));
        turmoil::repair(host_name(3), host_name(2));

        // The healed cluster must fully converge: one leader, and all of
        // {1, 2, 3} holding an identical log, applied state, and state
        // machine.
        let deadline = turmoil::elapsed() + Duration::from_secs(120);
        loop {
            let snapshots = cs.lock().unwrap().get_all_full_snapshots();
            match liveness::check_converged(&snapshots) {
                Ok(c) => {
                    assert_eq!(c.members, (1..=3).collect());
                    break;
                }
                Err(reason) => {
                    if turmoil::elapsed() > deadline {
                        panic!("cluster failed to converge after heal: {reason}");
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        Ok(())
    });

    sim.run().expect("scenario client failed");
}
