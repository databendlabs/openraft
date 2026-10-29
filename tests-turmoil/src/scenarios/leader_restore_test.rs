use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use openraft::EntryPayload;
use openraft::async_runtime::WatchReceiver;
use openraft::storage::RaftLogReader;

use super::wait_for;
use crate::cluster::ClusterState;
use crate::cluster::bounce_node;
use crate::cluster::crash_node;
use crate::cluster::host_name;
use crate::cluster::register_node_storage;
use crate::cluster::spawn_host;
use crate::liveness;
use crate::storage_model::StorageModel;
use crate::store::ValueMeta;
use crate::typ::*;

#[test]
fn restarted_leader_with_election_preserves_entry_identity() {
    leader_restart_preserves_entry_identity(false);
}

#[test]
fn restored_leader_preserves_entry_identity() {
    leader_restart_preserves_entry_identity(true);
}

/// Regression trap for openraft [#2167]: a leader that restarts after losing
/// an unflushed entry must not give the same log id to a new entry, because a
/// follower may already hold the lost entry under that log id.
///
/// Sequence: leader 1 stops flushing → partition node 3 → write X, which only
/// node 2 persists → power-loss crash of node 1, which loses X → restart node
/// 1 and heal → node 1 leads again, either resumed at startup by leader
/// restore or elected in a new term → write Y. Y must get a log id other than
/// X's, and the cluster must converge on Y.
///
/// [#2167]: https://github.com/databendlabs/openraft/issues/2167
fn leader_restart_preserves_entry_identity(restore: bool) {
    const SEED: u64 = 0x2167;
    let filter = tracing_subscriber::EnvFilter::from_default_env();
    let subscriber = tracing_subscriber::fmt().with_env_filter(filter);
    let subscriber = subscriber.with_test_writer().finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    // A round trip stays well within the AppendEntries timeout of 3/4 of `heartbeat_interval`.
    let mut sim = turmoil::Builder::new()
        .simulation_duration(Duration::from_secs(120))
        .min_message_latency(Duration::from_millis(1))
        .max_message_latency(Duration::from_millis(5))
        .rng_seed(SEED)
        .build();
    // Heartbeats keep the leader lease that every write requires. Only this test starts elections.
    let config = Arc::new(openraft::Config {
        enable_elect: false,
        enable_leader_restore: Some(restore),
        ..Default::default()
    });
    // A follower refuses to vote until `election_timeout_max` after the last heartbeat it received.
    let follower_lease = Duration::from_millis(config.election_timeout_max);
    let cluster = Arc::new(Mutex::new(ClusterState::new()));
    let nodes: BTreeMap<_, _> = (1..=3)
        .map(|id| {
            (id, Node {
                addr: format!("{}:9000", host_name(id)),
            })
        })
        .collect();
    let model = StorageModel {
        async_flush: true,
        flush_delay_ms: (1, 1),
        ..StorageModel::baseline()
    };
    for id in 1..=3 {
        register_node_storage(id, &cluster, &model, SEED + id);
        spawn_host(&mut sim, id, config.clone(), cluster.clone(), SEED, nodes.clone());
    }

    let armed = Arc::new(Mutex::new(None));
    let ready = armed.clone();
    let cs = cluster.clone();
    sim.client("before-crash", async move {
        tracing::info!("Wait for leader 1 and a converged cluster to fix the last entry before the fault");
        let (raft, base) = {
            let (leader_id, raft) = wait_for("initial leader", || cs.lock().unwrap().find_leader_entry()).await;
            assert_eq!(leader_id, 1);
            let base = wait_for("initial cluster convergence", || {
                let snapshots = cs.lock().unwrap().get_all_full_snapshots();
                let converged = liveness::check_converged(&snapshots);
                converged.ok()?;
                snapshots[0].1.sm.last_applied
            })
            .await;
            (raft, base)
        };

        tracing::info!(
            ?base,
            "Keep a new entry on follower 2 while preventing leader 1 from flushing it"
        );
        let lost = {
            let leader_store = cs.lock().unwrap().log_stores[&1].clone();
            leader_store.pause_flush(true);
            turmoil::partition(host_name(1), host_name(3));
            turmoil::partition(host_name(2), host_name(3));
            let request = Request {
                client_id: "scenario".to_string(),
                serial: 1,
                key: "key".to_string(),
                value: "before".to_string(),
            };
            raft.client_write_ff(request.clone(), None).await?;
            let log_id = wait_for("unflushed leader entry persisted on follower 2", || {
                let state = cs.lock().unwrap();
                let live = state.log_stores[&1].log_id_list();
                let durable = state.log_stores[&2].durable_log_id_list();
                let last = live.last().copied()?;
                if last.index <= base.index || durable.last() != Some(&last) {
                    return None;
                }
                Some(last)
            })
            .await;
            let durable = leader_store.durable_log_id_list();
            let last_durable = durable.last().copied();
            assert_eq!(last_durable, Some(base));
            let applied = cs.lock().unwrap().state_machines[&1].get_data().last_applied;
            assert_eq!(applied, Some(base));
            let mut follower_store = cs.lock().unwrap().log_stores[&2].clone();
            let entries = follower_store.try_get_log_entries(log_id.index..=log_id.index).await?;
            let expected = Entry {
                log_id,
                payload: EntryPayload::Normal(request),
            };
            assert_eq!(entries, vec![expected.clone()]);
            expected
        };
        *ready.lock().unwrap() = Some((base, lost));
        Ok(())
    });

    while armed.lock().unwrap().is_none() {
        let finished = sim.step().expect("failed to prepare the crash");
        let ready = armed.lock().unwrap().is_some();
        assert!(!finished || ready, "scenario exited before preparing the crash");
    }
    let (base, lost) = armed.lock().unwrap().take().unwrap();
    tracing::info!(
        ?base,
        ?lost,
        "Restart leader 1 immediately after losing its unflushed entry"
    );
    {
        crash_node(&mut sim, 1, &cluster, true);
        let store = cluster.lock().unwrap().log_stores[&1].clone();
        let recovered = store.log_id_list();
        let last = recovered.last().copied();
        assert_eq!(last, Some(base));
        store.pause_flush(false);
        bounce_node(&mut sim, 1);
        sim.repair(host_name(1), host_name(3));
        sim.repair(host_name(2), host_name(3));
    }

    let cs = cluster.clone();
    sim.client("after-crash", async move {
        let raft = wait_for("node 1 restarted", || cs.lock().unwrap().get_raft(1)).await;

        tracing::info!(
            ?follower_lease,
            "Elect node 1 if it has not resumed leadership once the followers' lease on its old term expires"
        );
        {
            tokio::time::sleep(follower_lease).await;
            let resumed = raft.metrics().borrow_watched().state.is_leader();
            if !resumed {
                raft.trigger().elect(false).await?;
            }
            wait_for("node 1 leading with a lease acknowledged by a quorum", || {
                let metrics = raft.metrics().borrow_watched().clone();
                let leading = metrics.state.is_leader() && metrics.last_quorum_acked.is_some();
                leading.then_some(())
            })
            .await;
        }

        tracing::info!(?lost, restore, "Require the next write to have a distinct log ID");
        let response = {
            let request = Request {
                client_id: "scenario".to_string(),
                serial: 2,
                key: "key".to_string(),
                value: "after".to_string(),
            };
            let response = raft.client_write(request).await?;
            let violations = cs.lock().unwrap().drain_log_violations();
            assert_eq!(violations, Vec::<String>::new());
            assert_ne!(response.log_id, lost.log_id);
            response
        };

        tracing::info!(log_id = %response.log_id, "Require all nodes to converge on the new value");
        {
            let converged = wait_for("cluster convergence after restart", || {
                let snapshots = cs.lock().unwrap().get_all_full_snapshots();
                let result = liveness::check_converged(&snapshots);
                result.ok()
            })
            .await;
            let expected_members = (1..=3).collect();
            assert_eq!(converged.members, expected_members);
            let data = cs.lock().unwrap().state_machines[&1].get_data().data;
            let expected = BTreeMap::from([("key".to_string(), ValueMeta {
                value: "after".to_string(),
                serial: 2,
                log_id: response.log_id,
            })]);
            assert_eq!(data, expected);
            let violations = cs.lock().unwrap().drain_log_violations();
            assert_eq!(violations, Vec::<String>::new());
        }
        Ok(())
    });
    sim.run().expect("leader restart violated entry identity or convergence");
}
