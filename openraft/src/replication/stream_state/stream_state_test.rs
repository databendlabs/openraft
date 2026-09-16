use std::fmt::Debug;
use std::io;
use std::ops::RangeBounds;
use std::sync::Arc;

use super::StreamState;
use crate::Config;
use crate::OptionalSend;
use crate::RaftLogReader;
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
                let (_io_tx, io_rx) = UTConfig::<()>::watch_channel(IOId::new_log_io(vote.clone(), range.last));
                let (_cancel_tx, cancel_rx) = UTConfig::<()>::watch_channel(());
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
                    },
                    payload: Some(Payload::Probe { log_id_range: range }),
                    inflight_id: Some(inflight_id),
                    backoff_consumer: BackoffState::new().consumer(),
                }
            };

            if empty_first {
                tracing::info!("--- an empty storage read must leave the probe pending");
                {
                    let request = state.next_request().await.unwrap().unwrap();
                    assert!(request.entries.is_empty());
                    assert_eq!(Some(log_id(1, 1, 52)), request.prev_log_id);
                    assert!(matches!(state.payload, Some(Payload::Probe { .. })));
                }
            }

            tracing::info!("--- the entry-carrying request contains only the storage-limited prefix");
            {
                let request = state.next_request().await.unwrap().unwrap();
                assert_eq!(Some(log_id(1, 1, 52)), request.prev_log_id);
                assert_eq!(vec![EntryOf::<UTConfig>::new_blank(log_id(1, 1, 53))], request.entries);
                assert_eq!(vec![(53, 61); if empty_first { 2 } else { 1 }], state.log_reader.reads);
            }

            tracing::info!("--- the stream ends without reading or sending the remaining seven entries");
            {
                let reads = state.log_reader.reads.len();
                assert!(state.next_request().await.unwrap().is_none());
                assert_eq!(reads, state.log_reader.reads.len());
            }
        }
    });
}
