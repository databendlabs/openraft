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
