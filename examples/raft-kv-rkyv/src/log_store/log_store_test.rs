use std::io;

use openraft::RaftLogReader;
use openraft::StorageError;
use openraft::alias::VoteOf;
use openraft::entry::RaftEntry;
use openraft::storage::RaftLogStorage;
use openraft::storage::RaftLogStorageExt;
use openraft::testing::log::StoreBuilder;
use openraft::testing::log::Suite;
use openraft::type_config::TypeConfigExt;
use openraft::vote::RaftLeaderId;
use tempfile::TempDir;

use super::LogStore;
use crate::ArchiveBuffer;
use crate::CommittedLeaderId;
use crate::Entry;
use crate::LogId;
use crate::Payload;
use crate::StateMachine;
use crate::TypeConfig;

struct Builder;

impl StoreBuilder<TypeConfig, LogStore, StateMachine, TempDir> for Builder {
    async fn build(&self) -> Result<(TempDir, LogStore, StateMachine), StorageError<TypeConfig>> {
        let directory = TempDir::new().map_err(|e| StorageError::write(TypeConfig::err_from_error(&e)))?;
        let store =
            LogStore::open(directory.path()).map_err(|e| StorageError::write(TypeConfig::err_from_error(&e)))?;
        Ok((directory, store, StateMachine::default()))
    }
}

#[tokio::test]
async fn storage_suite() -> anyhow::Result<()> {
    Suite::test_all(Builder).await?;
    Ok(())
}

#[tokio::test]
async fn reopen_recovers_metadata_conflicting_suffix_and_retained_archives() -> anyhow::Result<()> {
    let directory = TempDir::new()?;
    let mut store = LogStore::open(directory.path())?;
    let vote = VoteOf::<TypeConfig>::new_committed(2, 1);
    let id = |index| LogId::new(CommittedLeaderId::new(1, 1), index);

    tracing::info!("persist a vote, committed progress, and three archived entries");
    let retained = {
        store.save_vote(&vote).await?;
        let data = ArchiveBuffer::new("key", &"value".repeat(256))?;
        store
            .blocking_append((0..3).map(|index| Entry::new(id(index), Payload::Normal(data.clone()))))
            .await?;
        store.save_committed(Some(id(0))).await?;
        data
    };

    tracing::info!("truncate the conflicting suffix and replace it with a higher-term entry");
    let replacement = {
        store.truncate_after(Some(id(0))).await?;
        let next = LogId::new(CommittedLeaderId::new(2, 1), 1);
        store
            .blocking_append([Entry::new(
                next,
                Payload::Normal(ArchiveBuffer::new("next", "replacement")?),
            )])
            .await?;
        next
    };
    drop(store);

    tracing::info!("reopen independent archive allocations and verify all persisted metadata");
    let reopened_archive = {
        let mut store = LogStore::open(directory.path())?;
        assert_eq!(Some(vote), store.read_vote().await?);
        assert_eq!(Some(id(0)), store.read_committed().await?);
        assert_eq!(Some(replacement), store.get_log_state().await?.last_log_id);
        let entries = store.try_get_log_entries(..).await?;
        assert_eq!(2, entries.len());
        let Payload::Normal(reopened) = &entries[0].payload else {
            panic!("normal entry")
        };
        assert_eq!(retained.bytes(), reopened.bytes());
        assert_ne!(retained.bytes().as_ptr(), reopened.bytes().as_ptr());
        assert_eq!(0, reopened.bytes().as_ptr().align_offset(16));
        let reader = store.try_get_log_entries(0..1).await?;
        let Payload::Normal(again) = &reader[0].payload else {
            panic!("normal entry")
        };
        assert_eq!(reopened.bytes().as_ptr(), again.bytes().as_ptr());
        let reopened = reopened.clone();
        store.purge(id(0)).await?;
        assert_eq!(Some(id(0)), store.get_log_state().await?.last_purged_log_id);
        store.truncate_after(None).await?;
        reopened
    };

    tracing::info!("retained handles survive purge, file replacement, and closing the store");
    {
        assert_eq!("value".repeat(256), reopened_archive.archived().value.as_str());
        let mut store = LogStore::open(directory.path())?;
        let state = store.get_log_state().await?;
        assert_eq!(Some(id(0)), state.last_purged_log_id);
        assert_eq!(state.last_purged_log_id, state.last_log_id);
        assert!(store.try_get_log_entries(..).await?.is_empty());
        assert_eq!(Some(vote), store.read_vote().await?);
        assert_eq!(Some(id(0)), store.read_committed().await?);
    }
    Ok(())
}

#[tokio::test]
async fn interrupted_publication_never_acknowledges_and_requires_reopen() -> anyhow::Result<()> {
    for stage in [
        "partial_write",
        "before_file_sync",
        "after_file_sync",
        "after_rename",
        "after_directory_sync",
    ] {
        tracing::info!(stage, "interrupt image publication at a write/flush boundary");
        let directory = TempDir::new()?;
        let mut store = LogStore::open(directory.path())?;
        let old_vote = VoteOf::<TypeConfig>::new(1, 1);
        store.save_vote(&old_vote).await?;
        store.fail_at(stage).await;
        let id = LogId::new(CommittedLeaderId::new(1, 1), 0);
        let result =
            store.blocking_append([Entry::new(id, Payload::Normal(ArchiveBuffer::new("key", "value")?))]).await;
        assert!(result.is_err(), "must not acknowledge an interrupted append at {stage}");
        assert!(store.read_vote().await.is_err(), "uncertain store must be fenced");
        assert!(store.save_vote(&VoteOf::<TypeConfig>::new(9, 1)).await.is_err());
        drop(store);

        tracing::info!(
            stage,
            "recover the complete published generation, ignoring temporary bytes"
        );
        {
            let mut store = LogStore::open(directory.path())?;
            assert_eq!(Some(old_vote), store.read_vote().await?, "acknowledged vote survives");
            let entries = store.try_get_log_entries(..).await?;
            assert_eq!(
                usize::from(matches!(stage, "after_rename" | "after_directory_sync")),
                entries.len()
            );
            if let Some(entry) = entries.first() {
                let Payload::Normal(data) = &entry.payload else {
                    panic!("normal entry")
                };
                assert_eq!("value", data.archived().value.as_str());
            }
            store.save_vote(&VoteOf::<TypeConfig>::new(3, 1)).await?;
        }
    }
    Ok(())
}

#[tokio::test]
async fn exclusive_open_and_unpublished_first_generation() -> anyhow::Result<()> {
    let directory = TempDir::new()?;
    let mut store = LogStore::open(directory.path())?;
    assert!(LogStore::open(directory.path()).is_err());
    let clone = store.clone();
    store.fail_at("after_file_sync").await;
    assert!(store.save_vote(&VoteOf::<TypeConfig>::new(1, 1)).await.is_err());
    drop(store);
    assert!(
        LogStore::open(directory.path()).is_err(),
        "clones retain the writer lock"
    );
    drop(clone);
    let mut store = LogStore::open(directory.path())?;
    assert!(
        store.read_vote().await?.is_none(),
        "unpublished .next is not authoritative"
    );
    store.save_vote(&VoteOf::<TypeConfig>::new(2, 1)).await?;
    drop(store);
    assert_eq!(
        Some(VoteOf::<TypeConfig>::new(2, 1)),
        LogStore::open(directory.path())?.read_vote().await?
    );
    Ok(())
}

#[tokio::test]
async fn cancelled_update_keeps_cache_and_disk_in_one_generation() -> anyhow::Result<()> {
    let directory = TempDir::new()?;
    let mut store = LogStore::open(directory.path())?;
    let vote = VoteOf::<TypeConfig>::new(4, 1);
    let worker = store.clone();
    let (started, wait_started) = tokio::sync::oneshot::channel();
    let (release, wait_release) = std::sync::mpsc::channel();

    tracing::info!("cancel the async caller while its blocking update owns the store lock");
    {
        let task = tokio::spawn(async move {
            worker
                .update(move |state| {
                    state.vote = Some(vote);
                    started.send(()).unwrap();
                    wait_release.recv().unwrap();
                })
                .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), wait_started).await??;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        release.send(())?;
    }

    tracing::info!("verify the completed background write updates the cache before another publication");
    {
        assert_eq!(Some(vote), store.read_vote().await?);
        store.save_committed(None).await?;
        drop(store);
        assert_eq!(Some(vote), LogStore::open(directory.path())?.read_vote().await?);
    }
    Ok(())
}

#[test]
fn reopening_rejects_corruption_versions_and_invalid_archives() -> anyhow::Result<()> {
    use crate::disk_state::DiskState;

    let directory = TempDir::new()?;
    let data = ArchiveBuffer::new(&"key".repeat(64), &"value".repeat(64))?;
    let mut state = DiskState::default();
    state.entries.insert(
        0,
        Entry::new(
            LogId::new(CommittedLeaderId::new(1, 1), 0),
            Payload::Normal(data.clone()),
        ),
    );
    let valid = state.encode()?;
    let mut checksum_error = valid.clone();
    *checksum_error.last_mut().unwrap() ^= 1;
    let mut unknown_version = valid.clone();
    unknown_version[7] = b'2';
    let mut invalid_archive = valid.clone();
    let position = invalid_archive.windows(data.bytes().len()).position(|bytes| bytes == data.bytes()).unwrap();
    invalid_archive[position] = 0xff;
    let checksum = crc32fast::hash(&invalid_archive[12..]).to_le_bytes();
    invalid_archive[8..12].copy_from_slice(&checksum);
    let mut trailing = valid.clone();
    trailing.push(0);
    let checksum = crc32fast::hash(&trailing[12..]).to_le_bytes();
    trailing[8..12].copy_from_slice(&checksum);
    for bytes in [
        vec![],
        valid[..valid.len() - 1].to_vec(),
        checksum_error,
        unknown_version,
        invalid_archive,
        trailing,
    ] {
        std::fs::write(directory.path().join("log-state"), bytes)?;
        let error = LogStore::open(directory.path()).err().expect("reject damaged published image");
        assert_eq!(io::ErrorKind::InvalidData, error.kind());
    }
    std::fs::write(directory.path().join("log-state"), valid)?;
    assert!(LogStore::open(directory.path()).is_ok());
    Ok(())
}
