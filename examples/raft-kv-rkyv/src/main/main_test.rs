use openraft::RaftLogReader;
use openraft::RaftSnapshotBuilder;
use openraft::entry::RaftEntry;
use openraft::storage::RaftStateMachine;
use openraft::vote::RaftLeaderId;

use super::ArchiveBuffer;
use super::Cluster;
use super::Entry;
use super::LogId;
use super::Snapshot;
use super::StateMachine;
use super::codec::Codec;
use super::payload::Payload;

#[tokio::test]
async fn archived_data_survives_storage_replication_and_apply() -> anyhow::Result<()> {
    tracing::info!("create three voters with ticking disabled to exercise archive replication");
    let cluster = Cluster::new().await?;

    tracing::info!("submit one archive and retain its allocation for storage/apply identity checks");
    let (original, index) = {
        let original = ArchiveBuffer::new("key", &"value".repeat(1024))?;
        let response = cluster.nodes[&1].0.client_write(original.clone()).await?;
        (original, response.log_id.index())
    };

    tracing::info!(
        index,
        "wait for every voter to apply the entry received through the byte codec"
    );
    cluster.wait_applied(index).await?;

    tracing::info!(
        index,
        "verify logs and applied state share archives locally, not across nodes"
    );
    {
        for (id, (_, state, log)) in &cluster.nodes {
            let entries = log.clone().try_get_log_entries(index..=index).await?;
            let Payload::Normal(stored) = &entries[0].payload else {
                anyhow::bail!("expected archived normal entry on node {id}");
            };
            let applied = state.get("key").await.unwrap();
            assert_eq!(original.bytes(), stored.bytes());
            assert_eq!(stored.bytes().as_ptr(), applied.bytes().as_ptr());
            assert_eq!("value".repeat(1024), applied.archived().value.as_str());
            if *id == 1 {
                assert_eq!(original.bytes().as_ptr(), stored.bytes().as_ptr());
            } else {
                // A direct in-process call that merely clones the request would fail this check.
                assert_ne!(original.bytes().as_ptr(), stored.bytes().as_ptr());
            }
        }
    }
    cluster.shutdown().await
}

#[tokio::test]
async fn snapshot_restores_validated_archive_handles() -> anyhow::Result<()> {
    tracing::info!("apply an archive on three voters before building a snapshot");
    let cluster = Cluster::new().await?;
    let response = cluster.nodes[&1].0.client_write(ArchiveBuffer::new("key", "snapshot value")?).await?;
    cluster.wait_applied(response.log_id.index()).await?;

    tracing::info!("restore a versioned snapshot frame into a fresh state machine");
    {
        let mut source = cluster.nodes[&1].1.clone();
        let snapshot = source.build_snapshot().await?;
        let frame = Codec::encode(&(snapshot.meta.clone(), snapshot.snapshot.into_inner()))?;
        let (meta, bytes) = Codec::decode(&frame)?;
        let mut restored = StateMachine::default();
        restored.install_snapshot(&meta, std::io::Cursor::new(bytes)).await?;
        let data = restored.get("key").await.unwrap();
        assert_eq!("snapshot value", data.archived().value.as_str());
        let (applied, membership) = restored.applied_state().await?;
        assert_eq!(meta.last_log_id, applied);
        assert_eq!(meta.last_membership, membership);
        let retained: Snapshot = restored.get_current_snapshot().await?.unwrap();
        assert_eq!(meta, retained.meta);
    }
    cluster.shutdown().await
}

#[tokio::test]
async fn persistent_node_replays_after_restart_and_catches_up() -> anyhow::Result<()> {
    use openraft::storage::RaftLogStorage;

    let directory = tempfile::TempDir::new()?;
    tracing::info!("create three voters with persistent logs and automatic snapshots disabled");
    let mut cluster = Cluster::persistent(directory.path()).await?;
    let response = cluster.nodes[&1].0.client_write(ArchiveBuffer::new("before", "durable archive")?).await?;
    cluster.wait_applied(response.log_id.index()).await?;

    tracing::info!(
        index = response.log_id.index(),
        "retain an archive but drop the follower's log store and state"
    );
    let retained = {
        let value = cluster.nodes[&3].1.get("before").await.unwrap();
        let committed = cluster.nodes[&3].2.clone().read_committed().await?;
        assert_eq!(Some(response.log_id), committed);
        cluster.stop_node(3).await?;
        value
    };

    tracing::info!("commit another value while node 3 is offline");
    let offline = {
        let response = cluster.nodes[&1].0.client_write(ArchiveBuffer::new("offline", "catch-up value")?).await?;
        cluster.wait_applied(response.log_id.index()).await?;
        response.log_id
    };

    tracing::info!("reopen node 3 and verify committed replay before a new heartbeat");
    {
        cluster.restart_node(3, directory.path()).await?;
        cluster.nodes[&3]
            .0
            .wait(Some(std::time::Duration::from_secs(5)))
            .applied_index_at_least(Some(response.log_id.index()), "replay durable committed prefix")
            .await?;
        let replayed = cluster.nodes[&3].1.get("before").await.unwrap();
        assert_eq!(retained.bytes(), replayed.bytes());
        assert_ne!(retained.bytes().as_ptr(), replayed.bytes().as_ptr());
    }

    tracing::info!(
        index = offline.index(),
        "trigger replication and verify the restarted voter catches up"
    );
    {
        cluster.nodes[&1].0.trigger().heartbeat().await?;
        cluster.wait_applied(offline.index()).await?;
        let replayed = cluster.nodes[&3].1.get("offline").await.unwrap();
        assert_eq!("catch-up value", replayed.archived().value.as_str());
        cluster.nodes[&1].0.ensure_linearizable(openraft::ReadPolicy::ReadIndex).await?;
    }
    cluster.shutdown().await?;

    tracing::info!("reopen all nodes with fresh state machines and elect from recovered votes/logs");
    {
        let cluster = Cluster::persistent(directory.path()).await?;
        // A remembered leader ID is not sufficient: the recovered node must actually win
        // the new election before Cluster::persistent returns and accepts this write.
        let after = cluster.nodes[&1].0.client_write(ArchiveBuffer::new("after", "new campaign")?).await?;
        cluster.wait_applied(after.log_id.index()).await?;
        for (_, state, _) in cluster.nodes.values() {
            assert_eq!(
                "durable archive",
                state.get("before").await.unwrap().archived().value.as_str()
            );
            assert_eq!(
                "catch-up value",
                state.get("offline").await.unwrap().archived().value.as_str()
            );
            assert_eq!(
                "new campaign",
                state.get("after").await.unwrap().archived().value.as_str()
            );
        }
        cluster.shutdown().await?;
    }
    Ok(())
}

#[test]
fn changing_log_id_preserves_the_application_archive() -> anyhow::Result<()> {
    let data = ArchiveBuffer::new("key", "value")?;
    let first = LogId::new(openraft::impls::leader_id_adv::LeaderId::new(1, 1), 2);
    let next = LogId::new(openraft::impls::leader_id_adv::LeaderId::new(2, 1), 3);
    let mut entry = Entry::new(first, Payload::Normal(data.clone()));
    entry.set_log_id(next);
    assert_eq!(next, entry.log_id());
    let Payload::Normal(buffer) = entry.payload else {
        unreachable!();
    };
    assert_eq!(data.bytes().as_ptr(), buffer.bytes().as_ptr());
    assert_eq!(data.bytes(), buffer.bytes());
    Ok(())
}
