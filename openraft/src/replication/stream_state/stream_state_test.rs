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
use crate::log_id_range::LogIdRange;
use crate::progress::inflight_id::InflightId;
use crate::progress::stream_id::StreamId;
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
use crate::vote::raft_vote::RaftVoteExt;

/// Returns one entry per read, optionally preceded by an empty read.
struct PrefixStore {
    empty_first: bool,
    reads: Vec<(u64, u64)>,
}

impl RaftLogReader<UTConfig> for PrefixStore {
    async fn limited_get_log_entries(&mut self, start: u64, end: u64) -> Result<Vec<EntryOf<UTConfig>>, io::Error> {
        self.reads.push((start, end));
        if std::mem::take(&mut self.empty_first) {
            return Ok(vec![]);
        }
        Ok(vec![EntryOf::<UTConfig>::new_blank(log_id(1, 1, start))])
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

#[test]
fn probe_stops_after_storage_limited_prefix() {
    UTConfig::<()>::run(async {
        for empty_first in [false, true] {
            tracing::info!(
                empty_first,
                "--- probe (52, 60] with storage returning one entry at a time"
            );
            let mut state = {
                let vote = VoteOf::<UTConfig>::new_committed(1, 1).to_committed();
                let range = LogIdRange::new(Some(log_id(1, 1, 52)), Some(log_id(1, 1, 60)));
                let inflight_id = InflightId::new(1);
                let (_replicate_tx, replicate_rx) =
                    UTConfig::<()>::watch_channel(Replicate::new_probe(range, inflight_id));
                let (_committed_tx, committed_rx) = UTConfig::<()>::watch_channel(None);
                let (_purged_tx, purged_rx) = UTConfig::<()>::watch_channel(None);
                let (_io_tx, io_rx) = UTConfig::<()>::watch_channel(IOId::new_log_io(vote.clone(), range.last));
                let (_cancel_tx, cancel_rx) = UTConfig::<()>::watch_channel(());
                let (_backoff_reset_tx, backoff_reset_rx) = UTConfig::<()>::watch_channel(());
                let (_probe_tx, quorum_loss_probe_rx) = UTConfig::<()>::watch_channel(None);
                let (tx_notify, _rx_notify) = UTConfig::<()>::mpsc(1);

                StreamState::<UTConfig, PrefixStore> {
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
                        purged_rx,
                    },
                    log_reader: PrefixStore {
                        empty_first,
                        reads: vec![],
                    },
                    payload: Some(Payload::Probe { log_id_range: range }),
                    inflight_id: Some(inflight_id),
                    plan_obsolete: false,
                    backoff_consumer: BackoffState::new().consumer(backoff_reset_rx),
                    after_leader_check: None,
                }
            };

            tracing::info!("--- the first generated request completes the probe payload");
            {
                let request = state.next_request().await.unwrap().unwrap();
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
                assert!(state.next_request().await.unwrap().is_none());
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
                let (_replicate_tx, replicate_rx) = UTConfig::<()>::watch_channel(replicate);
                let (committed_tx, committed_rx) = UTConfig::<()>::watch_channel(Some(prefix_end));
                let (_purged_tx, purged_rx) = UTConfig::<()>::watch_channel(None);
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
                        purged_rx,
                    },
                    log_reader: PrefixStore {
                        empty_first: false,
                        reads: vec![],
                    },
                    payload: Some(Payload::Probe { log_id_range: range }),
                    inflight_id: Some(inflight_id),
                    plan_obsolete: false,
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
                let channels = (_replicate_tx, _purged_tx, _backoff_reset_tx, _probe_tx, _rx_notify);
                (state, publisher, channels)
            };

            let result = state.next_request().await;
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

/// Build a stream to node 2 that sends `payload` as `inflight_id`, while RaftCore's latest plan for
/// node 2 is `latest_inflight_id`, the log is submitted through index 20, and purged through
/// `purged`. The second value keeps the channels open.
fn new_stream(
    payload: Payload<UTConfig>,
    inflight_id: Option<InflightId>,
    latest_inflight_id: InflightId,
    purged: Option<LogIdOf<UTConfig>>,
) -> (StreamState<UTConfig, PrefixStore>, impl Sized) {
    let vote = VoteOf::<UTConfig>::new_committed(1, 1).to_committed();
    let latest = Replicate::new_logs_since(Some(log_id(1, 1, 1)), latest_inflight_id);
    let (replicate_tx, replicate_rx) = UTConfig::<()>::watch_channel(latest);
    let (committed_tx, committed_rx) = UTConfig::<()>::watch_channel(None);
    let submitted = IOId::new_log_io(vote.clone(), Some(log_id(1, 1, 20)));
    let (io_tx, io_rx) = UTConfig::<()>::watch_channel(submitted);
    let (purged_tx, purged_rx) = UTConfig::<()>::watch_channel(purged);
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
            purged_rx,
        },
        log_reader: PrefixStore {
            empty_first: false,
            reads: vec![],
        },
        payload: Some(payload),
        inflight_id,
        plan_obsolete: false,
        backoff_consumer: BackoffState::new().consumer(backoff_reset_rx),
        after_leader_check: None,
    };

    let channels = (
        replicate_tx,
        committed_tx,
        io_tx,
        purged_tx,
        cancel_tx,
        backoff_reset_tx,
        probe_tx,
        rx_notify,
    );
    (state, channels)
}

/// A `LogsSince` stream whose next entry is purged ends without reading the log, because
/// RaftCore purges no entry that a plan in flight still has to send.
#[test]
fn logs_since_ends_when_its_next_entry_is_purged() {
    UTConfig::<()>::run(async {
        let payload = Payload::LogsSince {
            prev: Some(log_id(1, 1, 5)),
        };

        tracing::info!("--- the log is purged through index 8: the entry after prev 5 is gone");
        {
            let inflight_id = InflightId::new(1);
            let (mut state, _channels) =
                new_stream(payload.clone(), Some(inflight_id), inflight_id, Some(log_id(1, 1, 8)));

            let request = state.next_request().await.unwrap();
            assert!(request.is_none(), "expect no request, got {:?}", request);
            assert!(state.plan_obsolete);
            assert!(state.log_reader.reads.is_empty());
        }

        tracing::info!("--- the log is purged through prev itself: the next entry is still there");
        {
            let inflight_id = InflightId::new(1);
            let (mut state, _channels) =
                new_stream(payload.clone(), Some(inflight_id), inflight_id, Some(log_id(1, 1, 5)));

            let request = state.next_request().await.unwrap().unwrap();
            assert_eq!(Some(log_id(1, 1, 5)), request.prev_log_id);
            assert_eq!(vec![EntryOf::<UTConfig>::new_blank(log_id(1, 1, 6))], request.entries);
            assert!(!state.plan_obsolete);
        }

        tracing::info!("--- an entry missing above the purge point is a storage hole, not a dropped plan");
        {
            let inflight_id = InflightId::new(1);
            let (mut state, _channels) = new_stream(payload, Some(inflight_id), inflight_id, Some(log_id(1, 1, 5)));
            state.log_reader.empty_first = true;

            let request = state.next_request().await.unwrap().unwrap();
            assert_eq!(Some(log_id(1, 1, 5)), request.prev_log_id);
            assert!(request.entries.is_empty());
            assert!(!state.plan_obsolete);
        }
    });
}

/// A `LogsSince` stream ends at its next request once RaftCore publishes another plan, even while
/// it still has entries to send.
#[test]
fn logs_since_ends_when_its_plan_is_replaced() {
    UTConfig::<()>::run(async {
        let payload = Payload::LogsSince {
            prev: Some(log_id(1, 1, 5)),
        };
        let (mut state, _channels) = new_stream(payload, Some(InflightId::new(1)), InflightId::new(2), None);

        let request = state.next_request().await.unwrap();
        assert!(request.is_none(), "expect no request, got {:?}", request);
        assert!(state.plan_obsolete);
        assert!(state.log_reader.reads.is_empty());
    });
}

/// A one-shot probe still sends its only request when it is not RaftCore's latest plan and its
/// `prev` lies below the purge point, as the entry-less probe that delivers a commit does.
#[test]
fn one_shot_probe_still_sends_below_the_purge_point() {
    UTConfig::<()>::run(async {
        let matching = Some(log_id(1, 1, 3));
        let payload = Payload::Probe {
            log_id_range: LogIdRange::new(matching, matching),
        };
        let (mut state, _channels) = new_stream(payload, None, InflightId::new(2), Some(log_id(1, 1, 8)));

        let request = state.next_request().await.unwrap().unwrap();
        assert_eq!(matching, request.prev_log_id);
        assert!(request.entries.is_empty());
        assert!(!state.plan_obsolete);
        assert!(state.next_request().await.unwrap().is_none());
    });
}
