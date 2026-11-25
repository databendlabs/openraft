use std::fmt::Debug;
use std::io;
use std::ops::RangeBounds;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use super::StreamState;
use crate::Config;
use crate::OptionalSend;
use crate::RaftLogReader;
use crate::async_runtime::WatchSender;
use crate::core::SharedReplicateBatch;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::entry::RaftEntry;
use crate::errors::ReplicationClosed;
use crate::log_id_range::LogIdRange;
use crate::progress::inflight_id::InflightId;
use crate::progress::stream_id::StreamId;
use crate::raft::AppendEntriesRequest;
use crate::raft_state::IOId;
use crate::replication::backoff_state::BackoffState;
use crate::replication::event_watcher::EventWatcher;
use crate::replication::payload::Payload;
use crate::replication::replicate::Replicate;
use crate::replication::replication_context::ReplicationContext;
use crate::storage::IOFlushed;
use crate::storage::LogState;
use crate::storage::RaftLogStorage;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::EntryOf;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::VoteOf;
use crate::type_config::alias::WatchSenderOf;
use crate::vote::raft_vote::RaftVoteExt;

/// Returns one entry per read, optionally preceded by an empty read.
struct PrefixStore {
    empty_first: bool,
    reads: Vec<(u64, u64)>,
    /// Runs inside the next read, standing for what RaftCore does while the read is running. It is
    /// given the index the read starts at and returns the index of the first entry the read finds,
    /// which is greater once a purge has removed entries.
    during_read: Option<Box<dyn FnOnce(u64) -> u64 + Send + Sync>>,
}

impl RaftLogReader<UTConfig> for PrefixStore {
    async fn limited_get_log_entries(&mut self, start: u64, end: u64) -> Result<Vec<EntryOf<UTConfig>>, io::Error> {
        self.reads.push((start, end));
        if std::mem::take(&mut self.empty_first) {
            return Ok(vec![]);
        }
        let first = match self.during_read.take() {
            Some(during_read) => during_read(start),
            None => start,
        };
        if first >= end {
            return Ok(vec![]);
        }
        Ok(vec![EntryOf::<UTConfig>::new_blank(log_id(1, 1, first))])
    }

    async fn try_get_log_entries<R>(&mut self, _range: R) -> Result<Vec<EntryOf<UTConfig>>, io::Error>
    where R: RangeBounds<u64> + Clone + Debug + OptionalSend {
        unreachable!("replication should use limited_get_log_entries")
    }

    async fn read_vote(&mut self) -> Result<Option<VoteOf<UTConfig>>, io::Error> {
        unreachable!()
    }
}

impl RaftLogStorage<UTConfig> for PrefixStore {
    type LogReader = Self;

    async fn get_log_state(&mut self) -> Result<LogState<UTConfig>, io::Error> {
        unreachable!()
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        unreachable!()
    }

    async fn save_vote(&mut self, _vote: &VoteOf<UTConfig>) -> Result<(), io::Error> {
        unreachable!()
    }

    async fn append<I>(&mut self, _entries: I, _callback: IOFlushed<UTConfig>) -> Result<(), io::Error>
    where
        I: IntoIterator<Item = EntryOf<UTConfig>> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        unreachable!()
    }

    async fn truncate_after(&mut self, _last_log_id: Option<LogIdOf<UTConfig>>) -> Result<(), io::Error> {
        unreachable!()
    }

    async fn purge(&mut self, _log_id: LogIdOf<UTConfig>) -> Result<(), io::Error> {
        unreachable!()
    }
}

impl StreamState<UTConfig, PrefixStore> {
    /// Exercise the plan and task lifetime with an open transport session.
    async fn next_request_in_session(&mut self) -> Result<Option<AppendEntriesRequest<UTConfig>>, ReplicationClosed> {
        let (_session_tx, mut session_rx) = UTConfig::<()>::watch_channel(());
        self.next_request(&mut session_rx).await
    }
}

#[test]
fn probe_stops_after_storage_limited_prefix() {
    UTConfig::<()>::run(async {
        for empty_first in [false, true] {
            tracing::info!(
                empty_first,
                "--- probe (52, 60] with storage returning one entry at a time"
            );
            // The stream ends once its cancel sender drops, as when RaftCore closes it: keep it.
            let (mut state, _cancel_tx) = {
                let vote = VoteOf::<UTConfig>::new_committed(1, 1).to_committed();
                let range = LogIdRange::new(Some(log_id(1, 1, 52)), Some(log_id(1, 1, 60)));
                let inflight_id = InflightId::new(1);
                let (_replicate_tx, replicate_rx) =
                    UTConfig::<()>::watch_channel(Some(Replicate::new_probe(range, inflight_id)));
                let (_committed_tx, committed_rx) = UTConfig::<()>::watch_channel(None);
                let (_io_tx, io_rx) = UTConfig::<()>::watch_channel(IOId::new_log_io(vote.clone(), range.last));
                let (cancel_tx, cancel_rx) = UTConfig::<()>::watch_channel(());
                let (_backoff_reset_tx, backoff_reset_rx) = UTConfig::<()>::watch_channel(());
                let (_probe_tx, quorum_loss_probe_rx) = UTConfig::<()>::watch_channel(None);
                let (tx_notify, _rx_notify) = UTConfig::<()>::mpsc(1);

                let state = StreamState::<UTConfig, PrefixStore> {
                    replication_context: ReplicationContext {
                        id: 1,
                        target: 2,
                        leader_vote: vote,
                        stream_id: StreamId::new(1),
                        config: Arc::new(Config {
                            max_payload_entries: 8,
                            ..Default::default()
                        }),
                        tx_notify,
                        cancel_rx,
                        replicate_batch: SharedReplicateBatch::new(),
                        quorum_loss_probe_rx,
                    },
                    event_watcher: EventWatcher {
                        replicate_rx,
                        committed_rx,
                        io_accepted_rx: io_rx.clone(),
                        io_submitted_rx: io_rx,
                    },
                    log_reader: PrefixStore {
                        empty_first,
                        reads: vec![],
                        during_read: None,
                    },
                    payload: Some(Payload::Probe { log_id_range: range }),
                    inflight_id: Some(inflight_id),
                    backoff_consumer: BackoffState::new().consumer(backoff_reset_rx),
                    after_leader_check: None,
                };
                (state, cancel_tx)
            };

            tracing::info!("--- the first generated request completes the probe payload");
            {
                let request = state.next_request_in_session().await.unwrap().unwrap();
                assert_eq!(Some(log_id(1, 1, 52)), request.prev_log_id);
                let expected_entries = if empty_first {
                    vec![]
                } else {
                    vec![EntryOf::<UTConfig>::new_blank(log_id(1, 1, 53))]
                };
                assert_eq!(expected_entries, request.entries);
                assert_eq!(vec![(53, 61)], state.log_reader.reads);
                assert_eq!(None, state.payload.as_ref());
            }

            tracing::info!("--- the stream ends after one request");
            {
                let reads = state.log_reader.reads.len();
                assert!(state.next_request_in_session().await.unwrap().is_none());
                assert_eq!(reads, state.log_reader.reads.len());
            }
        }
    });
}

/// An old stream must keep its old commit when leadership advances mid-poll (#2245).
#[test]
fn stale_stream_does_not_mix_votes_and_later_commits() {
    const PAUSE_TIMEOUT: Duration = Duration::from_secs(5);

    UTConfig::<()>::run(async {
        for advance_leadership in [false, true] {
            let old_vote = VoteOf::<UTConfig>::new_committed(1, 1);
            let prefix_end = log_id(1, 1, 8);
            let old_entry = log_id(1, 1, 9);
            let later_commit = log_id(3, 1, 13);
            tracing::info!(
                advance_leadership,
                "--- prepare the term-1 stream with an uncommitted entry at index 9"
            );
            let (mut state, publisher, _channels) = {
                let range = LogIdRange::new(Some(prefix_end), Some(old_entry));
                let inflight_id = InflightId::new(1);
                let replicate = Replicate::new_probe(range, inflight_id);
                let (_replicate_tx, replicate_rx) = UTConfig::<()>::watch_channel(Some(replicate));
                let (committed_tx, committed_rx) = UTConfig::<()>::watch_channel(Some(prefix_end));
                let accepted = IOId::new_log_io(old_vote.to_committed(), Some(old_entry));
                let (io_tx, io_rx) = UTConfig::<()>::watch_channel(accepted);
                let (cancel_tx, cancel_rx) = UTConfig::<()>::watch_channel(());
                let mut cancel_tx = Some(cancel_tx);
                let (_backoff_reset_tx, backoff_reset_rx) = UTConfig::<()>::watch_channel(());
                let (_probe_tx, quorum_loss_probe_rx) = UTConfig::<()>::watch_channel(None);
                let (tx_notify, _rx_notify) = UTConfig::<()>::mpsc(1);
                let (paused_tx, paused_rx) = mpsc::channel();
                let (resume_tx, resume_rx) = mpsc::channel();

                let state = StreamState::<UTConfig, PrefixStore> {
                    replication_context: ReplicationContext {
                        id: 1,
                        target: 4,
                        leader_vote: old_vote.to_committed(),
                        stream_id: StreamId::new(1),
                        config: Arc::new(Config::default()),
                        tx_notify,
                        cancel_rx,
                        replicate_batch: SharedReplicateBatch::new(),
                        quorum_loss_probe_rx,
                    },
                    event_watcher: EventWatcher {
                        replicate_rx,
                        committed_rx,
                        io_accepted_rx: io_rx.clone(),
                        io_submitted_rx: io_rx,
                    },
                    log_reader: PrefixStore {
                        empty_first: false,
                        reads: vec![],
                        during_read: None,
                    },
                    payload: Some(Payload::Probe { log_id_range: range }),
                    inflight_id: Some(inflight_id),
                    backoff_consumer: BackoffState::new().consumer(backoff_reset_rx),
                    after_leader_check: Some(Box::new(move || {
                        paused_tx.send(()).unwrap();
                        resume_rx.recv_timeout(PAUSE_TIMEOUT).expect("publisher must release the paused stream");
                    })),
                };

                let publisher = std::thread::spawn(move || {
                    paused_rx.recv_timeout(PAUSE_TIMEOUT).expect("next_request must reach the leadership check");
                    tracing::info!(
                        advance_leadership,
                        "--- publish accepted IO before the later leader commit"
                    );
                    {
                        if advance_leadership {
                            let vote = VoteOf::<UTConfig>::new_committed(2, 2).to_committed();
                            let log_id = log_id(2, 2, 9);
                            let accepted = IOId::new_log_io(vote, Some(log_id));
                            let published = io_tx.send_if_greater(accepted);
                            assert!(published);
                            drop(cancel_tx.take());

                            let vote = VoteOf::<UTConfig>::new_committed(3, 1).to_committed();
                            let accepted = IOId::new_log_io(vote, Some(later_commit));
                            let published = io_tx.send_if_greater(accepted);
                            assert!(published);
                            let published = committed_tx.send_if_greater(Some(later_commit));
                            assert!(published);
                        }
                        resume_tx.send(()).unwrap();
                        (cancel_tx, io_tx, committed_tx)
                    }
                });
                let channels = (_replicate_tx, _backoff_reset_tx, _probe_tx, _rx_notify);
                (state, publisher, channels)
            };

            let result = state.next_request_in_session().await;
            let _publishers = publisher.join().unwrap();
            let generated = result.unwrap();

            tracing::info!(
                advance_leadership,
                ?generated,
                "--- inspect the request produced by the leader's stream"
            );
            {
                let Some(request) = generated else {
                    assert!(advance_leadership, "an unchanged leader must produce a request");
                    continue;
                };
                assert_eq!(old_vote, request.vote);
                let expected_prev = Some(prefix_end);
                assert_eq!(expected_prev, request.prev_log_id);
                let expected_entries = vec![EntryOf::<UTConfig>::new_blank(old_entry)];
                assert_eq!(expected_entries, request.entries);
                let expected_commit = Some(prefix_end);
                assert_eq!(
                    expected_commit, request.leader_commit,
                    "next_request produced {request}"
                );
            }
        }
    });
}

/// Build a stream to node 2 that runs `payload` as `inflight_id`, while RaftCore's current plan for
/// node 2 has the inflight id `current`, or is dropped if `current` is `None`. The log is submitted
/// through index 20. Next to the stream, it returns the sender that publishes RaftCore's plan, the
/// sender whose drop closes the stream, and a value that keeps the other channels open.
fn new_stream(
    payload: Payload<UTConfig>,
    inflight_id: Option<InflightId>,
    current: Option<InflightId>,
) -> (
    StreamState<UTConfig, PrefixStore>,
    WatchSenderOf<UTConfig, Option<Replicate<UTConfig>>>,
    WatchSenderOf<UTConfig, ()>,
    impl Sized,
) {
    let vote = VoteOf::<UTConfig>::new_committed(1, 1).to_committed();
    let current = current.map(|id| Replicate::new_logs_since(Some(log_id(1, 1, 1)), id));
    let (replicate_tx, replicate_rx) = UTConfig::<()>::watch_channel(current);
    let (committed_tx, committed_rx) = UTConfig::<()>::watch_channel(None);
    let submitted = IOId::new_log_io(vote.clone(), Some(log_id(1, 1, 20)));
    let (io_tx, io_rx) = UTConfig::<()>::watch_channel(submitted);
    let (cancel_tx, cancel_rx) = UTConfig::<()>::watch_channel(());
    let (backoff_reset_tx, backoff_reset_rx) = UTConfig::<()>::watch_channel(());
    let (probe_tx, quorum_loss_probe_rx) = UTConfig::<()>::watch_channel(None);
    let (tx_notify, rx_notify) = UTConfig::<()>::mpsc(1);

    let state = StreamState::<UTConfig, PrefixStore> {
        replication_context: ReplicationContext {
            id: 1,
            target: 2,
            leader_vote: vote,
            stream_id: StreamId::new(1),
            config: Arc::new(Config::default()),
            tx_notify,
            cancel_rx,
            replicate_batch: SharedReplicateBatch::new(),
            quorum_loss_probe_rx,
        },
        event_watcher: EventWatcher {
            replicate_rx,
            committed_rx,
            io_accepted_rx: io_rx.clone(),
            io_submitted_rx: io_rx,
        },
        log_reader: PrefixStore {
            empty_first: false,
            reads: vec![],
            during_read: None,
        },
        payload: Some(payload),
        inflight_id,
        backoff_consumer: BackoffState::new().consumer(backoff_reset_rx),
        after_leader_check: None,
    };

    let channels = (committed_tx, io_tx, backoff_reset_tx, probe_tx, rx_notify);
    (state, replicate_tx, cancel_tx, channels)
}

/// A `LogsSince` stream ends at its next request once RaftCore clears its plan, although entries
/// are left to send and no other plan is published. A transport that polls the stream on a task of
/// its own may still poll it after the session ended.
#[test]
fn logs_since_ends_once_its_plan_is_cleared() {
    UTConfig::<()>::run(async {
        let inflight_id = InflightId::new(1);
        let clear = Replicate::new_clear(InflightId::new(2));

        tracing::info!("--- with its plan current, the stream sends entry 6 after prev 5");
        let payload = Payload::LogsSince {
            prev: Some(log_id(1, 1, 5)),
        };
        let (mut state, replicate_tx, _cancel_tx, _channels) =
            new_stream(payload, Some(inflight_id), Some(inflight_id));
        let reads = {
            let request = state.next_request_in_session().await.unwrap().unwrap();
            assert_eq!(Some(log_id(1, 1, 5)), request.prev_log_id);
            assert_eq!(vec![EntryOf::<UTConfig>::new_blank(log_id(1, 1, 6))], request.entries);
            state.log_reader.reads.len()
        };

        tracing::info!("--- RaftCore clears the plan while entries 7 to 20 are left: the stream ends");
        {
            replicate_tx.send(Some(clear.clone())).unwrap();

            let request = state.next_request_in_session().await.unwrap();
            assert!(request.is_none(), "expect no request, got {:?}", request);
            assert_eq!(reads, state.log_reader.reads.len(), "no read after the plan is cleared");
        }

        tracing::info!("--- a stream waiting for entries after 20 ends once RaftCore clears its plan");
        {
            let payload = Payload::LogsSince {
                prev: Some(log_id(1, 1, 20)),
            };
            let (mut state, replicate_tx, _cancel_tx, _channels) =
                new_stream(payload, Some(inflight_id), Some(inflight_id));

            let clear_plan = async {
                replicate_tx.send(Some(clear)).unwrap();
            };
            let (request, ()) = futures_util::future::join(state.next_request_in_session(), clear_plan).await;
            let request = request.unwrap();
            assert!(request.is_none(), "expect no request, got {:?}", request);
            assert!(state.log_reader.reads.is_empty());
        }
    });
}

/// A `LogsSince` stream ends at its next request once RaftCore publishes another plan, although
/// entries are left to send.
#[test]
fn logs_since_ends_once_its_plan_is_replaced() {
    UTConfig::<()>::run(async {
        let payload = Payload::LogsSince {
            prev: Some(log_id(1, 1, 5)),
        };
        let (mut state, _replicate_tx, _cancel_tx, _channels) =
            new_stream(payload, Some(InflightId::new(1)), Some(InflightId::new(2)));

        let request = state.next_request_in_session().await.unwrap();
        assert!(request.is_none(), "expect no request, got {:?}", request);
        assert!(state.log_reader.reads.is_empty());
    });
}

/// A one-shot probe sends its only request while RaftCore has no current plan, as the entry-less
/// probe that delivers a commit after a dropped plan does.
#[test]
fn one_shot_probe_sends_without_a_current_plan() {
    UTConfig::<()>::run(async {
        let matching = Some(log_id(1, 1, 3));
        let payload = Payload::Probe {
            log_id_range: LogIdRange::new(matching, matching),
        };
        let (mut state, _replicate_tx, _cancel_tx, _channels) = new_stream(payload, None, None);

        let request = state.next_request_in_session().await.unwrap().unwrap();
        assert_eq!(matching, request.prev_log_id);
        assert!(request.entries.is_empty());
        assert!(state.next_request_in_session().await.unwrap().is_none());
    });
}

/// A read can overlap the end of the plan it reads for, and its entries are then not sent.
///
/// Here the read finds its first entry at index 9, as if logs through 8 had been purged, and a
/// request with `prev_log_id` 5 must not carry it. RaftCore keeps the entries of a plan that it
/// clears, replaces or closes until the replication task no longer reads for it, so the read finds
/// them, but they belong to a plan RaftCore no longer runs.
#[test]
fn read_that_outlives_its_plan_is_not_sent() {
    #[derive(Clone, Copy, Debug)]
    enum PlanEnd {
        Clear,
        Replace,
        Close,
    }

    UTConfig::<()>::run(async {
        let inflight_id = InflightId::new(1);
        let prev = Some(log_id(1, 1, 5));
        let logs_since = Payload::LogsSince { prev };
        let probe = Payload::Probe {
            log_id_range: LogIdRange::new(prev, Some(log_id(1, 1, 9))),
        };

        for payload in [logs_since, probe] {
            for end in [PlanEnd::Clear, PlanEnd::Replace, PlanEnd::Close] {
                tracing::info!(%payload, ?end, "--- the plan of the stream ends during its read");
                {
                    let (mut state, replicate_tx, cancel_tx, _channels) =
                        new_stream(payload.clone(), Some(inflight_id), Some(inflight_id));
                    let during_read: Box<dyn FnOnce(u64) -> u64 + Send + Sync> = match end {
                        PlanEnd::Clear => Box::new(move |start| {
                            replicate_tx.send(Some(Replicate::new_clear(InflightId::new(2)))).unwrap();
                            start
                        }),
                        PlanEnd::Replace => Box::new(move |start| {
                            let next = Replicate::new_logs_since(Some(log_id(1, 1, 8)), InflightId::new(2));
                            replicate_tx.send(Some(next)).unwrap();
                            start
                        }),
                        PlanEnd::Close => Box::new(move |_start| {
                            drop(cancel_tx);
                            9
                        }),
                    };
                    state.log_reader.during_read = Some(during_read);

                    let request = state.next_request_in_session().await.unwrap();
                    assert!(request.is_none(), "expect no request, got {:?}", request);
                    assert_eq!(1, state.log_reader.reads.len());
                }
            }
        }
    });
}

/// Session EOF can interrupt the wait between reading a request and sending it.
#[test]
fn session_end_interrupts_backoff() {
    use futures_util::FutureExt;

    use crate::network::Backoff;

    UTConfig::<()>::run(async {
        let id = InflightId::new(1);
        let payload = Payload::LogsSince {
            prev: Some(log_id(1, 1, 5)),
        };
        let (mut state, _replicate_tx, _cancel_tx, _channels) = new_stream(payload, Some(id), Some(id));
        let (session_tx, mut session_rx) = UTConfig::<()>::watch_channel(());
        *state.backoff_consumer.inner.lock().unwrap() = Some(Backoff::new(std::iter::repeat(Duration::from_secs(60))));

        tracing::info!("--- end the session while the generated request waits for retry backoff");
        {
            let mut next = Box::pin(state.next_request(&mut session_rx));
            let pending = next.as_mut().now_or_never();
            assert!(pending.is_none(), "the request should wait in backoff");

            drop(session_tx);
            let stopped = UTConfig::<()>::timeout(Duration::from_secs(1), next).await;
            assert!(
                matches!(stopped, Ok(Ok(None))),
                "the canceled request yielded {stopped:?}"
            );
        }

        assert_eq!(vec![(6, 21)], state.log_reader.reads);
    });
}
