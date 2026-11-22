//! A replication task confirms a clear only once it reads nothing more for the plan RaftCore
//! dropped. RaftCore purges the logs the plan held back only after the confirmation. Likewise, a
//! task whose stream RaftCore closed returns only once it reads nothing more, and RaftCore purges
//! no log until it has returned.

use std::fmt::Debug;
use std::future::Future;
use std::io;
use std::ops::RangeBounds;
use std::sync::Arc;
use std::time::Duration;

use futures_util::Stream;
use futures_util::StreamExt;

use crate::Config;
use crate::OptionalSend;
use crate::RaftLogReader;
use crate::RaftNetworkFactory;
use crate::async_runtime::MpscReceiver;
use crate::async_runtime::WatchReceiver;
use crate::async_runtime::WatchSender;
use crate::base::BoxFuture;
use crate::base::BoxStream;
use crate::core::SharedReplicateBatch;
use crate::core::notification::Notification;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::entry::RaftEntry;
use crate::errors::RPCError;
use crate::errors::ReplicationClosed;
use crate::errors::StreamingError;
use crate::errors::Unreachable;
use crate::log_id_range::LogIdRange;
use crate::network::RPCOption;
use crate::network::RaftNetworkV2;
use crate::progress::inflight_id::InflightId;
use crate::progress::stream_id::StreamId;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendResult;
use crate::raft::SnapshotResponse;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::raft_state::IOId;
use crate::replication::ReplicationCore;
use crate::replication::event_watcher::EventWatcher;
use crate::replication::replicate::Replicate;
use crate::replication::replication_context::ReplicationContext;
use crate::replication::replication_progress::ReplicationProgress;
use crate::storage::IOFlushed;
use crate::storage::LogState;
use crate::storage::RaftLogStorage;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::EntryOf;
use crate::type_config::alias::JoinHandleOf;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::MpscReceiverOf;
use crate::type_config::alias::SnapshotOf;
use crate::type_config::alias::VoteOf;
use crate::type_config::alias::WatchReceiverOf;
use crate::type_config::alias::WatchSenderOf;
use crate::vote::raft_vote::RaftVoteExt;

type C = UTConfig;

const TEST_TIMEOUT: Duration = Duration::from_secs(1);

/// A log whose reads each find one entry, once `open` is `true`.
struct GatedLog {
    /// Counts the reads that have started.
    reads: Arc<WatchSenderOf<C, u64>>,
    open: WatchReceiverOf<C, bool>,
}

impl RaftLogReader<C> for GatedLog {
    async fn limited_get_log_entries(&mut self, start: u64, _end: u64) -> Result<Vec<EntryOf<C>>, io::Error> {
        let reads = *self.reads.borrow_watched() + 1;
        self.reads.send(reads).ok();

        self.open.wait_until_ge(&true).await.ok();
        Ok(vec![EntryOf::<C>::new_blank(log_id(1, 1, start))])
    }

    async fn try_get_log_entries<R>(&mut self, _range: R) -> Result<Vec<EntryOf<C>>, io::Error>
    where R: RangeBounds<u64> + Clone + Debug + OptionalSend {
        unreachable!("replication should use limited_get_log_entries")
    }

    async fn read_vote(&mut self) -> Result<Option<VoteOf<C>>, io::Error> {
        unreachable!()
    }
}

impl RaftLogStorage<C> for GatedLog {
    type LogReader = Self;

    async fn get_log_state(&mut self) -> Result<LogState<C>, io::Error> {
        unreachable!()
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        unreachable!()
    }

    async fn save_vote(&mut self, _vote: &VoteOf<C>) -> Result<(), io::Error> {
        unreachable!()
    }

    async fn append<I>(&mut self, _entries: I, _callback: IOFlushed<C>) -> Result<(), io::Error>
    where
        I: IntoIterator<Item = EntryOf<C>> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        unreachable!()
    }

    async fn truncate_after(&mut self, _last_log_id: Option<LogIdOf<C>>) -> Result<(), io::Error> {
        unreachable!()
    }

    async fn purge(&mut self, _log_id: LogIdOf<C>) -> Result<(), io::Error> {
        unreachable!()
    }
}

/// A transport that polls the request stream on a task of its own, as a gRPC client does, and
/// answers no request.
struct PullingNetwork {
    /// The polling task starts once this is `true`.
    poll: WatchReceiverOf<C, bool>,
    /// The number of requests the polling task has taken from the stream.
    pulled: Arc<WatchSenderOf<C, u64>>,
    /// Turns `true` once the request stream has ended.
    ended: Arc<WatchSenderOf<C, bool>>,
    /// Whether a session fails at once with an RPC error. Otherwise its response stream ends after
    /// its request stream does.
    fail: bool,
}

impl RaftNetworkFactory<C> for PullingNetwork {
    type Network = Self;

    async fn new_client(&mut self, _target: u64, _node: &()) -> Self {
        unreachable!("the test hands the network to the replication task")
    }
}

impl RaftNetworkV2<C> for PullingNetwork {
    type SnapshotData = ();

    fn stream_append<'s, S>(
        &'s mut self,
        mut input: S,
        _option: RPCOption,
    ) -> BoxFuture<'s, Result<BoxStream<'s, Result<AppendResult<C>, RPCError<C>>>, RPCError<C>>>
    where
        S: Stream<Item = AppendEntriesRequest<C>> + OptionalSend + Unpin + 'static,
    {
        let mut poll = self.poll.clone();
        let pulled = self.pulled.clone();
        let ended = self.ended.clone();
        #[allow(clippy::let_underscore_future)]
        let _ = C::spawn(async move {
            if poll.wait_until_ge(&true).await.is_err() {
                return;
            }

            while input.next().await.is_some() {
                let count = *pulled.borrow_watched() + 1;
                pulled.send(count).ok();
            }
            ended.send(true).ok();
        });

        let responses: BoxStream<'s, Result<AppendResult<C>, RPCError<C>>> = if self.fail {
            let error = RPCError::Unreachable(Unreachable::from_string("node 2 is down"));
            Box::pin(futures_util::stream::once(futures_util::future::ready(Err(error))))
        } else {
            let mut ended = self.ended.subscribe();
            let end = async move {
                ended.wait_until_ge(&true).await.ok();
            };
            Box::pin(futures_util::stream::once(end).filter_map(|()| futures_util::future::ready(None)))
        };
        Box::pin(futures_util::future::ready(Ok(responses)))
    }

    async fn append_entries(
        &mut self,
        _rpc: AppendEntriesRequest<C>,
        _option: RPCOption,
    ) -> Result<AppendResult<C>, RPCError<C>> {
        unreachable!("replication streams its requests")
    }

    async fn vote(&mut self, _rpc: VoteRequest<C>, _option: RPCOption) -> Result<VoteResponse<C>, RPCError<C>> {
        unreachable!()
    }

    async fn full_snapshot(
        &mut self,
        _vote: VoteOf<C>,
        _snapshot: SnapshotOf<C, ()>,
        _cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        _option: RPCOption,
    ) -> Result<SnapshotResponse<C>, StreamingError<C>> {
        unreachable!()
    }
}

/// The channels a test drives and watches a replication task to node 2 through.
struct Harness {
    replicate_tx: WatchSenderOf<C, Option<Replicate<C>>>,
    committed_tx: WatchSenderOf<C, Option<LogIdOf<C>>>,
    notifications: MpscReceiverOf<C, Notification<C>>,
    reads: WatchReceiverOf<C, u64>,
    open_reads: WatchSenderOf<C, bool>,
    poll: WatchSenderOf<C, bool>,
    pulled: WatchReceiverOf<C, u64>,
    ended: WatchReceiverOf<C, bool>,
    /// Resolves once the replication task returns.
    join: JoinHandleOf<C, Result<(), ReplicationClosed>>,
    /// Dropping it closes the stream, as RaftCore does.
    cancel_tx: WatchSenderOf<C, ()>,
}

/// Spawn a replication task to node 2, whose log is submitted through index 20 and matches it
/// through index 5. `open_reads` lets log reads return at once, `poll` lets the transport poll the
/// request stream at once, and `fail` makes each session fail, see [`PullingNetwork`]. Next to the
/// harness, it returns a value that keeps the other channels of the task open.
fn spawn_replication(open_reads: bool, poll: bool, fail: bool) -> (Harness, impl Sized) {
    let vote = VoteOf::<C>::new_committed(1, 1).to_committed();
    let (replicate_tx, replicate_rx) = C::watch_channel(None);
    let (committed_tx, committed_rx) = C::watch_channel(None);
    let submitted = IOId::new_log_io(vote.clone(), Some(log_id(1, 1, 20)));
    let (io_tx, io_rx) = C::watch_channel(submitted);
    let (cancel_tx, cancel_rx) = C::watch_channel(());
    let (backoff_reset_tx, backoff_reset_rx) = C::watch_channel(());
    let (probe_tx, quorum_loss_probe_rx) = C::watch_channel(None);
    let (tx_notify, notifications) = C::mpsc(16);
    let (reads_tx, reads) = C::watch_channel(0);
    let (open_reads_tx, open_reads_rx) = C::watch_channel(open_reads);
    let (poll_tx, poll_rx) = C::watch_channel(poll);
    let (pulled_tx, pulled) = C::watch_channel(0);
    let (ended_tx, ended) = C::watch_channel(false);

    let context = ReplicationContext {
        id: 1,
        target: 2,
        leader_vote: vote,
        stream_id: StreamId::new(1),
        config: Arc::new(Config::default()),
        tx_notify,
        cancel_rx,
        replicate_batch: SharedReplicateBatch::new(),
        quorum_loss_probe_rx,
    };
    let event_watcher = EventWatcher {
        replicate_rx,
        committed_rx,
        io_accepted_rx: io_rx.clone(),
        io_submitted_rx: io_rx,
    };
    let progress = ReplicationProgress {
        local_committed: None,
        remote_matched: Some(log_id(1, 1, 5)),
    };
    let network = PullingNetwork {
        poll: poll_rx,
        pulled: Arc::new(pulled_tx),
        ended: Arc::new(ended_tx),
        fail,
    };
    let log = GatedLog {
        reads: Arc::new(reads_tx),
        open: open_reads_rx,
    };

    let join = ReplicationCore::<C, PullingNetwork, GatedLog>::spawn(
        context,
        progress,
        network,
        log,
        event_watcher,
        backoff_reset_rx,
        tracing::Span::none(),
    );

    let harness = Harness {
        replicate_tx,
        committed_tx,
        notifications,
        reads,
        open_reads: open_reads_tx,
        poll: poll_tx,
        pulled,
        ended,
        join,
        cancel_tx,
    };
    (harness, (io_tx, backoff_reset_tx, probe_tx))
}

/// Wait for the next notification, which must be the confirmation of the clear `inflight_id`.
async fn expect_cleared(notifications: &mut MpscReceiverOf<C, Notification<C>>, inflight_id: InflightId) {
    let notification = C::timeout(TEST_TIMEOUT, notifications.recv()).await.unwrap().unwrap();
    let cleared = match &notification {
        Notification::ReplicateCleared { inflight_id: id, .. } => *id == inflight_id,
        _ => false,
    };
    assert!(
        cleared,
        "expect the clear {} confirmed, got: {}",
        inflight_id, notification
    );
}

/// Wait for the next notification, which must be the RPC error that ended the session of the plan
/// `inflight_id`.
async fn expect_failed(notifications: &mut MpscReceiverOf<C, Notification<C>>, inflight_id: InflightId) {
    let notification = C::timeout(TEST_TIMEOUT, notifications.recv()).await.unwrap().unwrap();
    let failed = match &notification {
        Notification::ReplicationProgress {
            progress,
            inflight_id: id,
            ..
        } => progress.result.is_err() && *id == Some(inflight_id),
        _ => false,
    };
    assert!(
        failed,
        "expect the RPC error of plan {}, got: {}",
        inflight_id, notification
    );
}

/// A read for a plan that RaftCore clears may still be running when the clear arrives. The
/// replication task confirms the clear only once the read has returned and the stream session has
/// ended, and it sends nothing the read returned.
#[test]
fn clear_is_confirmed_once_a_running_read_returns() {
    C::run(async {
        let (mut h, _channels) = spawn_replication(false, true, false);
        let plan = InflightId::new(1);
        let clear = InflightId::new(2);

        tracing::info!("--- a plan for the logs after index 5 starts a read, which the log holds");
        {
            h.replicate_tx.send(Some(Replicate::new_logs_since(Some(log_id(1, 1, 5)), plan))).unwrap();
            C::timeout(TEST_TIMEOUT, h.reads.wait_until_ge(&1)).await.unwrap().unwrap();
        }

        tracing::info!("--- RaftCore clears the plan: no confirmation while the read runs");
        {
            h.replicate_tx.send(Some(Replicate::new_clear(clear))).unwrap();

            C::sleep(Duration::from_millis(100)).await;
            let notification = h.notifications.try_recv();
            assert!(notification.is_err(), "expect no notification while the read runs");
        }

        tracing::info!("--- the read returns: the stream sends nothing, and the task confirms the clear");
        {
            h.open_reads.send(true).unwrap();
            expect_cleared(&mut h.notifications, clear).await;

            C::timeout(TEST_TIMEOUT, h.ended.wait_until_ge(&true)).await.unwrap().unwrap();
            assert_eq!(0, *h.pulled.borrow_watched(), "the stream ends without a request");
            assert_eq!(1, *h.reads.borrow_watched());
        }
    });
}

/// After a session fails, a transport that polls the request stream on a task of its own may still
/// start a read for the plan while the replication task waits for RaftCore. The task confirms the
/// clear that follows only once that read returns.
#[test]
fn clear_is_confirmed_once_a_read_that_outlived_its_session_returns() {
    C::run(async {
        let (mut h, _channels) = spawn_replication(false, false, true);
        let plan = InflightId::new(1);
        let clear = InflightId::new(2);

        tracing::info!("--- a plan for the logs after index 5 starts a session, which fails");
        {
            h.replicate_tx.send(Some(Replicate::new_logs_since(Some(log_id(1, 1, 5)), plan))).unwrap();
            expect_failed(&mut h.notifications, plan).await;
        }

        tracing::info!(
            "--- with the task waiting for RaftCore, the transport polls the stream, whose read the log holds"
        );
        {
            // Let the task go back to waiting for RaftCore after the error, so that the read starts
            // while it waits.
            C::sleep(Duration::from_millis(50)).await;
            h.poll.send(true).unwrap();
            C::timeout(TEST_TIMEOUT, h.reads.wait_until_ge(&1)).await.unwrap().unwrap();
        }

        tracing::info!("--- RaftCore clears the plan: no confirmation while the read runs");
        {
            h.replicate_tx.send(Some(Replicate::new_clear(clear))).unwrap();

            C::sleep(Duration::from_millis(100)).await;
            let notification = h.notifications.try_recv();
            assert!(notification.is_err(), "expect no notification while the read runs");
        }

        tracing::info!("--- the read returns: the stream sends nothing, and the task confirms the clear");
        {
            h.open_reads.send(true).unwrap();
            expect_cleared(&mut h.notifications, clear).await;

            C::timeout(TEST_TIMEOUT, h.ended.wait_until_ge(&true)).await.unwrap().unwrap();
            assert_eq!(0, *h.pulled.borrow_watched(), "the stream ends without a request");
            assert_eq!(1, *h.reads.borrow_watched());
        }
    });
}

/// Once the replication task confirms a clear, the request stream of the cleared plan reads
/// nothing, also when the transport polls it only after the session ended. The plan here is a
/// probe, for which the stream does not check RaftCore's current plan before it reads.
#[test]
fn stream_reads_nothing_once_the_clear_is_confirmed() {
    C::run(async {
        let (mut h, _channels) = spawn_replication(true, false, true);
        let plan = InflightId::new(1);
        let clear = InflightId::new(2);

        tracing::info!(
            "--- a probe of the entries after index 5 starts a session, which fails before the transport polls"
        );
        {
            let probe = LogIdRange::new(Some(log_id(1, 1, 5)), Some(log_id(1, 1, 9)));
            h.replicate_tx.send(Some(Replicate::new_probe(probe, plan))).unwrap();
            expect_failed(&mut h.notifications, plan).await;
        }

        tracing::info!("--- RaftCore clears the plan, and the task confirms it");
        {
            h.replicate_tx.send(Some(Replicate::new_clear(clear))).unwrap();
            expect_cleared(&mut h.notifications, clear).await;
        }

        tracing::info!("--- the transport polls the stream only now: it ends without a read");
        {
            h.poll.send(true).unwrap();

            C::timeout(TEST_TIMEOUT, h.ended.wait_until_ge(&true)).await.unwrap().unwrap();
            assert_eq!(0, *h.pulled.borrow_watched(), "the stream ends without a request");
            assert_eq!(0, *h.reads.borrow_watched());
        }
    });
}

/// A commit probe whose session ends without an answer is resumed. The task does not replace it
/// with the plan it ran before, which RaftCore did not publish again and may have finished since:
/// here a clear that the task already confirmed.
#[test]
fn unfinished_commit_probe_is_resumed() {
    C::run(async {
        let (mut h, _channels) = spawn_replication(true, true, false);
        let clear = InflightId::new(1);

        tracing::info!("--- RaftCore clears the plan for node 2, and the task confirms it");
        {
            h.replicate_tx.send(Some(Replicate::new_clear(clear))).unwrap();
            expect_cleared(&mut h.notifications, clear).await;
        }

        tracing::info!("--- index 5 commits: the task sends a commit probe, which gets no answer");
        {
            h.committed_tx.send(Some(log_id(1, 1, 5))).unwrap();
            C::timeout(TEST_TIMEOUT, h.pulled.wait_until_ge(&1)).await.unwrap().unwrap();
        }

        tracing::info!(
            "--- the task resends the probe, and does not confirm clear {} again",
            clear
        );
        {
            C::timeout(TEST_TIMEOUT, h.pulled.wait_until_ge(&2)).await.unwrap().unwrap();

            if let Ok(notification) = h.notifications.try_recv() {
                panic!("expect no notification, got: {}", notification);
            }
        }
    });
}

/// When RaftCore closes the stream, a read that the transport started on a task of its own may
/// still be running. The replication task returns only once that read has returned, and the stream
/// reads nothing more, because RaftCore purges the logs the task might read once it has returned.
#[test]
fn closed_task_returns_once_a_running_read_returns() {
    C::run(async {
        let (mut h, _channels) = spawn_replication(false, false, true);
        let plan = InflightId::new(1);

        tracing::info!("--- a plan's session fails, then the transport polls the stream, whose read the log holds");
        {
            h.replicate_tx.send(Some(Replicate::new_logs_since(Some(log_id(1, 1, 5)), plan))).unwrap();
            expect_failed(&mut h.notifications, plan).await;

            // Let the task go back to waiting for RaftCore after the error, so that the read starts
            // while it waits.
            C::sleep(Duration::from_millis(50)).await;
            h.poll.send(true).unwrap();
            C::timeout(TEST_TIMEOUT, h.reads.wait_until_ge(&1)).await.unwrap().unwrap();
        }

        tracing::info!("--- RaftCore closes the stream: the task does not return while the read runs");
        {
            drop(h.replicate_tx);
            drop(h.cancel_tx);

            let res = C::timeout(Duration::from_millis(100), &mut h.join).await;
            assert!(res.is_err(), "expect the task to wait for the read, got: {:?}", res);
        }

        tracing::info!("--- the read returns: the task returns, and the stream reads nothing more");
        {
            h.open_reads.send(true).unwrap();
            let res = C::timeout(TEST_TIMEOUT, h.join).await.unwrap();
            assert!(res.is_ok(), "the task returned without panicking: {:?}", res);

            C::timeout(TEST_TIMEOUT, h.ended.wait_until_ge(&true)).await.unwrap().unwrap();
            assert_eq!(1, *h.reads.borrow_watched());
        }
    });
}
