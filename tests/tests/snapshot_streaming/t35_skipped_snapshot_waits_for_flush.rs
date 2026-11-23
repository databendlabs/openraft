use std::collections::VecDeque;
use std::fmt::Debug;
use std::io;
use std::io::Cursor;
use std::ops::RangeBounds;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;
use openraft::Config;
use openraft::LogState;
use openraft::OptionalSend;
use openraft::Raft;
use openraft::RaftLogReader;
use openraft::Vote;
use openraft::alias::EntryOf;
use openraft::alias::LogIdOf;
use openraft::alias::SnapshotMetaOf;
use openraft::alias::SnapshotOf;
use openraft::alias::VoteOf;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::SnapshotResponse;
use openraft::storage::IOFlushed;
use openraft::storage::RaftLogStorage;
use openraft::testing::blank_ent;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::MemLogStore;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::WAIT_TIMEOUT;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// A log store that stores each append at once but holds its flush until the test releases it.
#[derive(Clone)]
struct HeldLogStore {
    inner: Arc<MemLogStore>,
    held: Arc<Mutex<VecDeque<IOFlushed<TypeConfig>>>>,
}

impl HeldLogStore {
    fn held(&self) -> usize {
        self.held.lock().unwrap().len()
    }

    /// Complete the oldest held append.
    fn flush_one(&self) {
        let callback = self.held.lock().unwrap().pop_front().expect("an append is held");
        callback.io_completed(Ok(()));
    }
}

impl RaftLogReader<TypeConfig> for HeldLogStore {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self,
        range: RB,
    ) -> Result<Vec<EntryOf<TypeConfig>>, io::Error> {
        self.inner.try_get_log_entries(range).await
    }

    async fn read_vote(&mut self) -> Result<Option<VoteOf<TypeConfig>>, io::Error> {
        self.inner.read_vote().await
    }
}

impl RaftLogStorage<TypeConfig> for HeldLogStore {
    type LogReader = Arc<MemLogStore>;

    async fn get_log_state(&mut self) -> Result<LogState<TypeConfig>, io::Error> {
        self.inner.get_log_state().await
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.inner.get_log_reader().await
    }

    async fn save_vote(&mut self, vote: &VoteOf<TypeConfig>) -> Result<(), io::Error> {
        self.inner.save_vote(vote).await
    }

    async fn save_committed(&mut self, committed: Option<LogIdOf<TypeConfig>>) -> Result<(), io::Error> {
        self.inner.save_committed(committed).await
    }

    async fn read_committed(&mut self) -> Result<Option<LogIdOf<TypeConfig>>, io::Error> {
        self.inner.read_committed().await
    }

    async fn append<I>(&mut self, entries: I, callback: IOFlushed<TypeConfig>) -> Result<(), io::Error>
    where
        I: IntoIterator<Item = EntryOf<TypeConfig>> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        self.inner.append(entries, IOFlushed::noop()).await?;
        self.held.lock().unwrap().push_back(callback);
        Ok(())
    }

    async fn truncate_after(&mut self, last_log_id: Option<LogIdOf<TypeConfig>>) -> Result<(), io::Error> {
        self.inner.truncate_after(last_log_id).await
    }

    async fn purge(&mut self, log_id: LogIdOf<TypeConfig>) -> Result<(), io::Error> {
        self.inner.purge(log_id).await
    }
}

/// A follower does not install a snapshot whose logs it has committed, but it answers the snapshot
/// only once those logs are flushed: the answer tells the leader that the follower stores them.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn skipped_snapshot_waits_for_flush() -> Result<()> {
    let config = Config {
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    // The router serves only as the network factory: this follower sends no RPC.
    let router = RaftRouter::new(config.clone())?;
    let (inner, state_machine) = openraft_memstore::new_mem_store();
    let log_store = HeldLogStore {
        inner,
        held: Default::default(),
    };
    let config = Arc::new(config.validate()?);
    let raft = Raft::new(1, config, router, log_store.clone(), state_machine).await?;
    let vote = Vote::new_committed(1, 0);

    tracing::info!("--- append logs 0 and 1 with commit 1, while the store holds the flush");
    let append = {
        let request = AppendEntriesRequest {
            vote,
            prev_log_id: None,
            entries: vec![blank_ent::<TypeConfig>(1, 0, 0), blank_ent::<TypeConfig>(1, 0, 1)],
            leader_commit: Some(log_id(1, 0, 1)),
        };
        let r = raft.clone();
        let append = TypeConfig::spawn(async move { r.append_entries(request).await });

        let committed = log_id(1, 0, 1);
        raft.wait(Some(WAIT_TIMEOUT))
            .metrics(
                |m| m.local_committed.as_ref() == Some(&committed),
                "log 1 is committed locally",
            )
            .await?;
        assert_eq!(1, log_store.held(), "the append of logs 0 and 1 waits to flush");
        append
    };

    tracing::info!("--- a snapshot up to log 1 is not installed, and its answer waits for the flush");
    let mut answer = {
        let snapshot = SnapshotOf::<TypeConfig, Cursor<Vec<u8>>> {
            meta: SnapshotMetaOf::<TypeConfig> {
                last_log_id: Some(log_id(1, 0, 1)),
                last_membership: Default::default(),
            },
            snapshot: Cursor::new(vec![]),
        };
        let r = raft.clone();
        let mut answer = TypeConfig::spawn(async move { r.install_full_snapshot(vote, snapshot).await });

        let waiting = TypeConfig::timeout(Duration::from_millis(200), &mut answer).await;
        assert!(waiting.is_err(), "the answer waits while logs 0 and 1 are not flushed");
        answer
    };

    tracing::info!("--- flush logs 0 and 1: the snapshot and the append are answered");
    {
        log_store.flush_one();

        let response = TypeConfig::timeout(WAIT_TIMEOUT, &mut answer).await???;
        assert_eq!(SnapshotResponse::new(vote), response);
        let reply = TypeConfig::timeout(WAIT_TIMEOUT, append).await???;
        assert_eq!(Ok(Some(log_id(1, 0, 1))), reply);
    }

    raft.shutdown().await?;
    Ok(())
}
