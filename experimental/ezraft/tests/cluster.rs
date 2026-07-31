//! Cluster-level tests driving EzRaft through its public API only: real HTTP
//! between nodes, in-memory persistence that survives a node instance so a
//! restart reads exactly what the previous incarnation persisted.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::io;
use std::io::Cursor;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use ezraft::EzApp;
use ezraft::EzConfig;
use ezraft::EzEntry;
use ezraft::EzMeta;
use ezraft::EzRaft;
use ezraft::EzSnapshot;
use ezraft::EzSnapshotMeta;
use ezraft::EzStorage;
use ezraft::Loaded;
use ezraft::Persist;
use serde::Deserialize;
use serde::Serialize;

#[derive(Serialize, Deserialize, Debug, Clone, derive_more::Display)]
enum Request {
    #[display("Set({key})")]
    Set { key: String, value: String },
    #[display("Get({key})")]
    Get { key: String },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
struct Response {
    value: Option<String>,
}

fn set(key: &str, value: &str) -> Request {
    Request::Set {
        key: key.into(),
        value: value.into(),
    }
}

fn get(key: &str) -> Request {
    Request::Get { key: key.into() }
}

/// KV app; the framework owns the value, so tests read the applied state
/// back through `EzRaft::read` to compare the whole state instead of
/// sampling it key by key.
#[derive(Default, Serialize, Deserialize)]
struct KvSm {
    data: BTreeMap<String, String>,
}

#[async_trait]
impl EzApp for KvSm {
    type Request = Request;
    type Response = Response;

    async fn apply(&mut self, req: Request) -> Response {
        match req {
            Request::Set { key, value } => {
                self.data.insert(key, value);
                Response { value: None }
            }
            Request::Get { key } => Response {
                value: self.data.get(&key).cloned(),
            },
        }
    }

    fn read(&self, key: &str) -> Option<serde_json::Value> {
        self.data.get(key).map(|value| serde_json::Value::String(value.clone()))
    }
}

/// What one node has "written to disk"; kept outside the storage instance so a
/// restarted node starts from it, exactly like a process reading real files.
#[derive(Default)]
struct Disk {
    meta: EzMeta,
    /// Log entries as serialized bytes, the same shape a real disk would hold.
    logs: BTreeMap<u64, Vec<u8>>,
    snapshot: Option<(EzSnapshotMeta, Vec<u8>)>,
}

#[derive(Clone, Default)]
struct MemStorage {
    disk: Arc<Mutex<Disk>>,
}

#[async_trait]
impl EzStorage<KvSm> for MemStorage {
    async fn load(&mut self) -> io::Result<Loaded> {
        let disk = self.disk.lock().unwrap();
        let snapshot = disk.snapshot.as_ref().map(|(meta, data)| EzSnapshot {
            meta: meta.clone(),
            snapshot: Cursor::new(data.clone()),
        });
        Ok(Loaded {
            meta: disk.meta.clone(),
            snapshot,
        })
    }

    async fn persist(&mut self, op: Persist<KvSm>) -> io::Result<()> {
        let mut disk = self.disk.lock().unwrap();
        match op {
            Persist::Meta(meta) => disk.meta = meta,
            Persist::LogEntry(entry) => {
                disk.logs.insert(entry.log_id.1, serde_json::to_vec(&entry)?);
            }
            Persist::Snapshot(snapshot) => {
                disk.snapshot = Some((snapshot.meta, snapshot.snapshot.into_inner()));
            }
            Persist::DeleteLogs { from, to } => disk.logs.retain(|&index, _| !(from..to).contains(&index)),
        }
        Ok(())
    }

    async fn read_logs(&mut self, start: u64, end: u64) -> io::Result<Vec<EzEntry<KvSm>>> {
        let disk = self.disk.lock().unwrap();
        (start..end)
            .map(|index| {
                let data =
                    disk.logs.get(&index).ok_or_else(|| io::Error::other(format!("missing log entry {}", index)))?;
                Ok(serde_json::from_slice(data)?)
            })
            .collect()
    }
}

/// Grab a free port; the listener is dropped so EzRaft can bind it.
fn free_addr() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().to_string()
}

/// Short heartbeat so elections and failovers finish in test time.
fn config() -> EzConfig {
    EzConfig {
        heartbeat_interval: Duration::from_millis(100),
        ..EzConfig::default()
    }
}

/// Upper bound for every cluster-state wait in these tests
const WAIT: Option<Duration> = Some(Duration::from_secs(30));

fn expected_map(range: std::ops::Range<u32>) -> BTreeMap<String, String> {
    range.map(|i| (format!("k{}", i), format!("v{}", i))).collect()
}

/// Joining must be all it takes to become a voter, and the promoted voters
/// must keep the cluster alive when the founding leader dies.
#[tokio::test(flavor = "multi_thread")]
async fn join_promotes_to_voter_and_cluster_survives_leader_death() -> io::Result<()> {
    let addr_a = free_addr();
    let a = EzRaft::create(&addr_a, KvSm::default(), MemStorage::default(), config()).await?;
    tokio::spawn({
        let a = a.clone();
        async move { a.serve().await }
    });
    a.inner()
        .wait(WAIT)
        .metrics(|m| m.current_leader == Some(0), "founding node leads")
        .await
        .map_err(io::Error::other)?;

    let addr_b = free_addr();
    let b = EzRaft::join(&addr_b, &addr_a, KvSm::default(), MemStorage::default(), config()).await?;
    tokio::spawn({
        let b = b.clone();
        async move { b.serve().await }
    });

    let addr_c = free_addr();
    let c = EzRaft::join(&addr_c, &addr_a, KvSm::default(), MemStorage::default(), config()).await?;
    tokio::spawn({
        let c = c.clone();
        async move { c.serve().await }
    });

    // Every node must see the final uniform config with all three voters before
    // the leader may die: a joint config still counts the founding node in its
    // old majority, so killing it mid-change would legitimately lose quorum.
    let voters = BTreeSet::from([0, b.node_id(), c.node_id()]);
    for node in [&a, &b, &c] {
        node.inner()
            .wait(WAIT)
            .metrics(
                |m| *m.membership_config.membership().get_joint_config() == [voters.clone()],
                "every joined node promoted to voter",
            )
            .await
            .map_err(io::Error::other)?;
    }

    assert_eq!(Response { value: None }, a.write(set("k1", "v1")).await?);

    // A direct read serves from local memory - no consensus round, no log
    // entry: the app's `read` answers the key from its own map.
    let value: serde_json::Value = reqwest::get(format!("http://{}/api/read?key=k1", addr_a))
        .await
        .map_err(io::Error::other)?
        .json()
        .await
        .map_err(io::Error::other)?;
    assert_eq!(serde_json::json!("v1"), value);

    // Kill the leader; the two remaining voters still form a quorum.
    assert!(a.is_leader());
    a.inner().shutdown().await.map_err(io::Error::other)?;

    b.inner()
        .wait(WAIT)
        .metrics(
            |m| matches!(m.current_leader, Some(id) if id != 0),
            "a surviving node takes over",
        )
        .await
        .map_err(io::Error::other)?;

    // The new leader accepts writes (reached from a follower via forwarding)
    // and still has the data acknowledged before the failover.
    assert_eq!(Response { value: None }, b.write(set("k2", "v2")).await?);
    assert_eq!(
        Response {
            value: Some("v1".into())
        },
        b.write(get("k1")).await?
    );
    assert_eq!(
        Response {
            value: Some("v2".into())
        },
        b.write(get("k2")).await?
    );

    Ok(())
}

/// A promotion interrupted by the death of the node that admitted the joiner
/// must be finished by the next leader: promotion belongs to the leader role,
/// not to the task that handled the join.
#[tokio::test(flavor = "multi_thread")]
async fn next_leader_promotes_orphaned_learner() -> io::Result<()> {
    let addr_a = free_addr();
    let a = EzRaft::create(&addr_a, KvSm::default(), MemStorage::default(), config()).await?;
    tokio::spawn({
        let a = a.clone();
        async move { a.serve().await }
    });
    a.inner()
        .wait(WAIT)
        .metrics(|m| m.current_leader == Some(0), "founding node leads")
        .await
        .map_err(io::Error::other)?;

    let addr_b = free_addr();
    let b = EzRaft::join(&addr_b, &addr_a, KvSm::default(), MemStorage::default(), config()).await?;
    tokio::spawn({
        let b = b.clone();
        async move { b.serve().await }
    });

    let addr_c = free_addr();
    let c = EzRaft::join(&addr_c, &addr_a, KvSm::default(), MemStorage::default(), config()).await?;
    tokio::spawn({
        let c = c.clone();
        async move { c.serve().await }
    });

    let voters = BTreeSet::from([0, b.node_id(), c.node_id()]);
    for node in [&a, &b, &c] {
        node.inner()
            .wait(WAIT)
            .metrics(
                |m| *m.membership_config.membership().get_joint_config() == [voters.clone()],
                "three voters before the fourth joins",
            )
            .await
            .map_err(io::Error::other)?;
    }

    // D joins but its server does not start: it cannot catch up, so it stays a
    // learner, holding its promotion open past the founding leader's death.
    let addr_d = free_addr();
    let d = EzRaft::join(&addr_d, &addr_a, KvSm::default(), MemStorage::default(), config()).await?;
    let d_id = d.node_id();
    a.inner()
        .wait(WAIT)
        .metrics(
            |m| m.membership_config.membership().learner_ids().any(|id| id == d_id),
            "joiner registered as learner",
        )
        .await
        .map_err(io::Error::other)?;

    a.inner().shutdown().await.map_err(io::Error::other)?;
    b.inner()
        .wait(WAIT)
        .metrics(
            |m| matches!(m.current_leader, Some(id) if id != 0),
            "a surviving node takes over",
        )
        .await
        .map_err(io::Error::other)?;

    // Only now does D serve and catch up. The node that admitted it is dead,
    // so only the new leader can complete the promotion.
    tokio::spawn({
        let d = d.clone();
        async move { d.serve().await }
    });

    let final_voters = BTreeSet::from([0, b.node_id(), c.node_id(), d_id]);
    b.inner()
        .wait(WAIT)
        .metrics(
            |m| m.membership_config.membership().voter_ids().collect::<BTreeSet<_>>() == final_voters,
            "next leader promotes the orphaned learner",
        )
        .await
        .map_err(io::Error::other)?;

    Ok(())
}

/// A snapshot must land on disk when built, and a restarted node must rebuild
/// the full state from that snapshot plus the log entries after it.
#[tokio::test(flavor = "multi_thread")]
async fn snapshot_survives_restart() -> io::Result<()> {
    let addr = free_addr();
    let storage = MemStorage::default();

    let a = EzRaft::create(&addr, KvSm::default(), storage.clone(), config()).await?;
    a.inner()
        .wait(WAIT)
        .metrics(|m| m.current_leader == Some(0), "single node leads")
        .await
        .map_err(io::Error::other)?;

    for i in 0..10 {
        a.write(set(&format!("k{}", i), &format!("v{}", i))).await?;
    }

    a.inner().trigger().snapshot().await.map_err(io::Error::other)?;
    a.inner()
        .wait(WAIT)
        .metrics(|m| m.snapshot.is_some(), "snapshot built")
        .await
        .map_err(io::Error::other)?;

    // The snapshot must be on disk with the full applied state: a restart
    // reads only the disk, so an unpersisted snapshot is lost data.
    {
        let disk = storage.disk.lock().unwrap();
        let (meta, data) = disk.snapshot.as_ref().expect("snapshot persisted to storage");
        let snapshot_state: KvSm = serde_json::from_slice(data)?;
        assert_eq!(expected_map(0..10), snapshot_state.data);
        assert!(meta.last_log_id.is_some());
    }

    // More writes after the snapshot: the restart must replay these on top.
    for i in 10..15 {
        a.write(set(&format!("k{}", i), &format!("v{}", i))).await?;
    }

    a.inner().shutdown().await.map_err(io::Error::other)?;
    drop(a);

    // Restart on the same disk with an empty state machine.
    let restarted = EzRaft::create(&addr, KvSm::default(), storage.clone(), config()).await?;
    restarted
        .inner()
        .wait(WAIT)
        .metrics(|m| m.current_leader == Some(0), "restarted node leads")
        .await
        .map_err(io::Error::other)?;
    restarted
        .inner()
        .wait(WAIT)
        .metrics(
            |m| m.last_applied.map(|log_id| log_id.index) == m.last_log_index,
            "log tail replayed",
        )
        .await
        .map_err(io::Error::other)?;

    // After the linearizable barrier, a local read must serve every write
    // acknowledged before the shutdown.
    restarted.linearizable().await?;
    assert_eq!(expected_map(0..15), restarted.read(|app| app.data.clone()).await);

    // And the restarted node serves reads over the rebuilt state.
    assert_eq!(
        Response {
            value: Some("v14".into())
        },
        restarted.write(get("k14")).await?
    );

    Ok(())
}

/// A node that joins after the leader purged its log can only be brought up by
/// a full snapshot over the network; the join must still end in a voter with
/// the complete state.
#[tokio::test(flavor = "multi_thread")]
async fn lagging_joiner_catches_up_from_snapshot() -> io::Result<()> {
    let addr_a = free_addr();
    let a = EzRaft::create(&addr_a, KvSm::default(), MemStorage::default(), config()).await?;
    tokio::spawn({
        let a = a.clone();
        async move { a.serve().await }
    });
    a.inner()
        .wait(WAIT)
        .metrics(|m| m.current_leader == Some(0), "founding node leads")
        .await
        .map_err(io::Error::other)?;

    for i in 0..10 {
        a.write(set(&format!("k{}", i), &format!("v{}", i))).await?;
    }

    // Snapshot, then purge every covered entry: whoever joins now cannot be
    // caught up by log replay.
    a.inner().trigger().snapshot().await.map_err(io::Error::other)?;
    let snapshot_index = a
        .inner()
        .wait(WAIT)
        .metrics(|m| m.snapshot.is_some(), "snapshot built")
        .await
        .map_err(io::Error::other)?
        .snapshot
        .unwrap()
        .index;
    a.inner().trigger().purge_log(snapshot_index).await.map_err(io::Error::other)?;
    a.inner()
        .wait(WAIT)
        .metrics(
            |m| m.purged.map(|log_id| log_id.index) == Some(snapshot_index),
            "log purged up to the snapshot",
        )
        .await
        .map_err(io::Error::other)?;

    let addr_b = free_addr();
    let b = EzRaft::join(&addr_b, &addr_a, KvSm::default(), MemStorage::default(), config()).await?;
    tokio::spawn({
        let b = b.clone();
        async move { b.serve().await }
    });

    let voters = BTreeSet::from([0, b.node_id()]);
    b.inner()
        .wait(WAIT)
        .metrics(
            |m| *m.membership_config.membership().get_joint_config() == [voters.clone()],
            "snapshot-fed joiner promoted to voter",
        )
        .await
        .map_err(io::Error::other)?;

    // The whole pre-purge state must have arrived through the snapshot.
    assert_eq!(expected_map(0..10), b.read(|app| app.data.clone()).await);

    // And the pair keeps working past the transfer.
    assert_eq!(Response { value: None }, b.write(set("k10", "v10")).await?);
    assert_eq!(
        Response {
            value: Some("v0".into())
        },
        b.write(get("k0")).await?
    );

    Ok(())
}

/// A demo-sized write load must drive the whole persist lifecycle on its own:
/// with `snapshot_interval` configured, a snapshot reaches storage and the log
/// entries it covers are deleted, all without a manual trigger.
#[tokio::test(flavor = "multi_thread")]
async fn automatic_snapshot_purges_old_logs() -> io::Result<()> {
    let addr = free_addr();
    let storage = MemStorage::default();

    let config = EzConfig {
        snapshot_interval: 5,
        ..config()
    };
    let a = EzRaft::create(&addr, KvSm::default(), storage.clone(), config).await?;
    a.inner()
        .wait(WAIT)
        .metrics(|m| m.current_leader == Some(0), "single node leads")
        .await
        .map_err(io::Error::other)?;

    for i in 0..12 {
        a.write(set(&format!("k{}", i), &format!("v{}", i))).await?;
    }

    let metrics = a
        .inner()
        .wait(WAIT)
        .metrics(
            |m| m.snapshot.is_some() && m.purged.is_some(),
            "snapshot built and log purged by the interval policy alone",
        )
        .await
        .map_err(io::Error::other)?;

    let disk = storage.disk.lock().unwrap();
    let (meta, data) = disk.snapshot.as_ref().expect("snapshot persisted to storage");
    assert!(meta.last_log_id.is_some());
    let snapshot_state: KvSm = serde_json::from_slice(data)?;
    assert!(!snapshot_state.data.is_empty());

    // DeleteLogs must have reached storage: every entry at or below the purged
    // index is gone, and entries above it survive.
    let purged_index = metrics.purged.unwrap().index;
    let min_kept = *disk.logs.keys().next().expect("entries above the purge point remain");
    assert!(
        min_kept > purged_index,
        "min kept index {} must be above purged index {}",
        min_kept,
        purged_index
    );

    Ok(())
}
