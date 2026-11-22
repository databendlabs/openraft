use std::io;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use maplit::btreeset;
use openraft::Config;
use openraft::LogState;
use openraft::OptionalSend;
use openraft::Raft;
use openraft::ServerState;
use openraft::alias::EntryOf;
use openraft::alias::LogIdOf;
use openraft::alias::OneshotReceiverOf;
use openraft::alias::OneshotSenderOf;
use openraft::alias::VoteOf;
use openraft::async_runtime::WatchReceiver;
use openraft::errors::Fatal;
use openraft::errors::RaftError;
use openraft::storage::IOFlushed;
use openraft::storage::RaftLogStorage;
use openraft::type_config::OneshotSender;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::MemLogStore;
use crate::fixtures::RaftRouter;
use crate::fixtures::WAIT_TIMEOUT;
use crate::fixtures::timeout;
use crate::fixtures::ut_harness;

/// How long a test watches `initialize()` to show that it has not returned.
const PENDING: Duration = Duration::from_millis(500);

/// The ends of a [`HeldLogStore`]'s gates that the test holds.
struct Gates {
    /// Receives the flush callback of the first append.
    flush: OneshotReceiverOf<TypeConfig, IOFlushed<TypeConfig>>,

    /// Resolves once the first `save_vote()` is entered.
    vote_entered: OneshotReceiverOf<TypeConfig, ()>,

    /// Lets the first `save_vote()` continue.
    vote_release: OneshotSenderOf<TypeConfig, ()>,
}

/// A log store that hands the flush callback of its first append to the test, and holds its first
/// vote save until the test releases it.
///
/// The held append is readable at once, as `RaftLogStorage::append()` requires. The vote save is
/// held too: once a later I/O completes, `RaftCore` counts the earlier ones as flushed.
struct HeldLogStore {
    inner: MemLogStore,
    flush: Mutex<Option<OneshotSenderOf<TypeConfig, IOFlushed<TypeConfig>>>>,
    vote_entered: Mutex<Option<OneshotSenderOf<TypeConfig, ()>>>,
    vote_release: Mutex<Option<OneshotReceiverOf<TypeConfig, ()>>>,
}

impl HeldLogStore {
    fn new(inner: MemLogStore) -> (Self, Gates) {
        let (flush_tx, flush) = TypeConfig::oneshot();
        let (entered_tx, vote_entered) = TypeConfig::oneshot();
        let (vote_release, release_rx) = TypeConfig::oneshot();

        let store = Self {
            inner,
            flush: Mutex::new(Some(flush_tx)),
            vote_entered: Mutex::new(Some(entered_tx)),
            vote_release: Mutex::new(Some(release_rx)),
        };
        let gates = Gates {
            flush,
            vote_entered,
            vote_release,
        };
        (store, gates)
    }
}

impl RaftLogStorage<TypeConfig> for HeldLogStore {
    type LogReader = MemLogStore;

    async fn get_log_state(&mut self) -> Result<LogState<TypeConfig>, io::Error> {
        self.inner.get_log_state().await
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.inner.clone()
    }

    async fn save_vote(&mut self, vote: &VoteOf<TypeConfig>) -> Result<(), io::Error> {
        let release = self.vote_release.get_mut().unwrap().take();
        if let Some(release) = release {
            let entered = self.vote_entered.get_mut().unwrap().take().unwrap();
            entered.send(()).ok();
            // A dropped sender releases the vote save too.
            release.await.ok();
        }
        self.inner.save_vote(vote).await
    }

    async fn append<I>(&mut self, entries: I, callback: IOFlushed<TypeConfig>) -> Result<(), io::Error>
    where
        I: IntoIterator<Item = EntryOf<TypeConfig>> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        let Some(flush) = self.flush.get_mut().unwrap().take() else {
            return self.inner.append(entries, callback).await;
        };
        self.inner.append(entries, IOFlushed::noop()).await?;
        let sent = flush.send(callback);
        assert!(sent.is_ok(), "the test keeps the flush receiver");
        Ok(())
    }

    async fn truncate_after(&mut self, last_log_id: Option<LogIdOf<TypeConfig>>) -> Result<(), io::Error> {
        self.inner.truncate_after(last_log_id).await
    }

    async fn purge(&mut self, log_id: LogIdOf<TypeConfig>) -> Result<(), io::Error> {
        self.inner.purge(log_id).await
    }
}

/// `initialize()` returns only after the membership log at index 0 is flushed.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn initialize_waits_for_the_membership_log_flush() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            enable_tick: false,
            enable_heartbeat: false,
            enable_elect: false,
            ..Default::default()
        }
        .validate()?,
    );
    let router = RaftRouter::new(config.as_ref().clone())?;
    let (log_store, sm) = openraft_memstore::new_mem_store();
    let (log_store, gates) = HeldLogStore::new(log_store);
    let n1 = Raft::new(1, config, router.clone(), log_store, sm).await?;

    tracing::info!("--- initialize node 1; hold the flush of log 0 and the vote save of the election after it");
    let (init_tx, mut init_rx) = TypeConfig::oneshot();
    let flush = {
        let n1 = n1.clone();
        TypeConfig::spawn(async move {
            init_tx.send(n1.initialize(btreeset! {1}).await).ok();
        });
        let flush = TypeConfig::timeout(WAIT_TIMEOUT, gates.flush).await??;
        TypeConfig::timeout(WAIT_TIMEOUT, gates.vote_entered).await??;
        flush
    };

    tracing::info!("--- initialize() does not return while log 0 is not flushed");
    {
        let got = TypeConfig::timeout(PENDING, &mut init_rx).await;
        assert!(
            got.is_err(),
            "initialize() returned before log 0 was flushed: {:?}",
            got
        );
    }

    tracing::info!("--- flush log 0 and release the vote save: initialize() returns Ok");
    {
        flush.io_completed(Ok(()));
        gates.vote_release.send(()).ok();

        let got = TypeConfig::timeout(WAIT_TIMEOUT, init_rx).await??;
        assert_eq!(Ok(()), got);
        n1.wait(timeout()).applied_index(Some(1), "node 1 commits its blank log").await?;
    }

    n1.shutdown().await?;
    Ok(())
}

/// A failed flush of the membership log fails `initialize()` with the storage error.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn initialize_fails_when_the_membership_log_flush_fails() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            enable_tick: false,
            enable_heartbeat: false,
            enable_elect: false,
            ..Default::default()
        }
        .validate()?,
    );
    let router = RaftRouter::new(config.as_ref().clone())?;
    let (log_store, sm) = openraft_memstore::new_mem_store();
    let (log_store, gates) = HeldLogStore::new(log_store);
    let n1 = Raft::new(1, config, router.clone(), log_store, sm).await?;

    tracing::info!("--- initialize node 1; hold the flush of log 0 and the vote save of the election after it");
    let (init_tx, mut init_rx) = TypeConfig::oneshot();
    let flush = {
        let n1 = n1.clone();
        TypeConfig::spawn(async move {
            init_tx.send(n1.initialize(btreeset! {1}).await).ok();
        });
        let flush = TypeConfig::timeout(WAIT_TIMEOUT, gates.flush).await??;
        TypeConfig::timeout(WAIT_TIMEOUT, gates.vote_entered).await??;
        flush
    };

    tracing::info!("--- fail the flush of log 0 while the vote save is held");
    {
        flush.io_completed(Err(io::Error::other("injected flush failure")));

        // The failure reaches `RaftCore` ahead of the vote save's completion.
        let got = TypeConfig::timeout(PENDING, &mut init_rx).await;
        assert!(
            got.is_err(),
            "initialize() returned while the vote save was held: {:?}",
            got
        );
    }

    tracing::info!("--- release the vote save: initialize() returns the storage error");
    {
        gates.vote_release.send(()).ok();

        let got = TypeConfig::timeout(WAIT_TIMEOUT, init_rx).await??;
        assert!(
            matches!(got, Err(RaftError::Fatal(Fatal::StorageError(_)))),
            "initialize() must fail: {:?}",
            got
        );
    }

    Ok(())
}

/// With its peers unreachable, a node still gets `Ok` from `initialize()`: the local flush of log 0
/// is enough, and no election quorum is needed.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn initialize_needs_no_election_quorum() -> anyhow::Result<()> {
    let config = Config {
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    router.new_raft_node(1).await?;
    let n1 = router.get_raft_handle(&1)?;

    tracing::info!("--- initialize node 1 with voters 2 and 3, which never start");
    {
        TypeConfig::timeout(WAIT_TIMEOUT, n1.initialize(btreeset! {1, 2, 3})).await??;
    }

    tracing::info!("--- node 1 campaigns, but it cannot win or commit log 0");
    {
        n1.wait(timeout()).state(ServerState::Candidate, "node 1 campaigns").await?;

        let metrics = n1.metrics().borrow_watched().clone();
        assert_eq!(None, metrics.current_leader);
        assert_eq!(None, metrics.last_applied);
    }

    Ok(())
}
