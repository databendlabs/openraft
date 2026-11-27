use std::collections::BTreeSet;
use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use futures_util::FutureExt;

use super::HeartbeatWorkersHandle;
use crate::Config;
use crate::OptionalSend;
use crate::RaftNetworkFactory;
use crate::async_runtime::MpscReceiver;
use crate::async_runtime::MpscSender;
use crate::core::heartbeat::event::HeartbeatEvent;
use crate::core::notification::Notification;
use crate::engine::TargetProgress;
use crate::engine::testing::UTConfig;
use crate::engine::testing::UTLeaderId;
use crate::errors::RPCError;
use crate::errors::ReplicationClosed;
use crate::errors::StreamingError;
use crate::impls::Vote;
use crate::network::RPCOption;
use crate::network::RaftNetworkV2;
use crate::progress::entry::ProgressEntry;
use crate::progress::stream_id::StreamId;
use crate::proposer::SendStamp;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendResult;
use crate::raft::SnapshotResponse;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::type_config::OneshotSender;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::CommittedVoteOf;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::MpscReceiverOf;
use crate::type_config::alias::MpscSenderOf;
use crate::type_config::alias::OneshotReceiverOf;
use crate::type_config::alias::OneshotSenderOf;
use crate::type_config::alias::SnapshotOf;
use crate::type_config::alias::VoteOf;
use crate::vote::raft_vote::RaftVoteExt;

/// The node metadata is a `String` standing for an address, so that a changed node is visible.
type C = UTConfig<String>;

const TEST_TIMEOUT: Duration = Duration::from_secs(1);

/// Long enough that no heartbeat RPC times out during a test.
const HEARTBEAT_MS: u64 = 3_600_000;

/// The `election_timeout_max` of a test whose worker stops with a pending RPC: the worker drops
/// the RPC once it has passed.
const STOP_LIMIT_MS: u64 = 50;

/// A heartbeat request, with the node of the client that sent it.
type Sent = (String, AppendEntriesRequest<C>);

/// A network factory that records each heartbeat client it creates, and the requests they send.
#[derive(Clone)]
struct RecordingFactory {
    /// The `(target, node)` of each created client, in creation order.
    created: Arc<Mutex<Vec<(u64, String)>>>,

    /// Completes when the client of the same position in `created` is dropped.
    dropped: Arc<Mutex<Vec<Option<OneshotReceiverOf<C, ()>>>>>,

    /// Clients created from these nodes leave every request pending.
    blocking: Arc<Mutex<BTreeSet<String>>>,

    sent: MpscSenderOf<C, Sent>,
}

impl RecordingFactory {
    fn new() -> (Self, MpscReceiverOf<C, Sent>) {
        let (sent, sent_rx) = C::mpsc(16);
        let factory = Self {
            created: Default::default(),
            dropped: Default::default(),
            blocking: Default::default(),
            sent,
        };
        (factory, sent_rx)
    }

    fn created(&self) -> Vec<(u64, String)> {
        self.created.lock().unwrap().clone()
    }

    /// The drop notification of the client created at position `index`.
    fn dropped(&self, index: usize) -> OneshotReceiverOf<C, ()> {
        self.dropped.lock().unwrap()[index].take().unwrap()
    }
}

impl RaftNetworkFactory<C> for RecordingFactory {
    type Network = RecordingClient;

    async fn new_client(&mut self, _target: u64, _node: &String) -> RecordingClient {
        unreachable!("heartbeat workers create their clients with new_heartbeat_client()")
    }

    async fn new_heartbeat_client(&mut self, target: u64, node: &String) -> RecordingClient {
        self.created.lock().unwrap().push((target, node.clone()));
        let (on_drop, dropped) = C::oneshot();
        self.dropped.lock().unwrap().push(Some(dropped));
        RecordingClient {
            node: node.clone(),
            block: self.blocking.lock().unwrap().contains(node),
            sent: self.sent.clone(),
            on_drop: Some(on_drop),
        }
    }
}

struct RecordingClient {
    node: String,
    block: bool,
    sent: MpscSenderOf<C, Sent>,
    on_drop: Option<OneshotSenderOf<C, ()>>,
}

impl Drop for RecordingClient {
    fn drop(&mut self) {
        if let Some(on_drop) = self.on_drop.take() {
            on_drop.send(()).ok();
        }
    }
}

impl RaftNetworkV2<C> for RecordingClient {
    type SnapshotData = ();

    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<C>,
        _option: RPCOption,
    ) -> Result<AppendResult<C>, RPCError<C>> {
        self.sent.send((self.node.clone(), rpc)).await.ok();
        if self.block {
            std::future::pending::<()>().await;
        }
        Ok(Ok(None))
    }

    async fn vote(&mut self, _rpc: VoteRequest<C>, _option: RPCOption) -> Result<VoteResponse<C>, RPCError<C>> {
        unreachable!("heartbeat workers send no votes")
    }

    async fn full_snapshot(
        &mut self,
        _vote: VoteOf<C>,
        _snapshot: SnapshotOf<C, ()>,
        _cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        _option: RPCOption,
    ) -> Result<SnapshotResponse<C>, StreamingError<C>> {
        unreachable!("heartbeat workers send no snapshots")
    }
}

fn progress(target: u64, node: &str, stream_id: u64) -> TargetProgress<C> {
    TargetProgress {
        target_node: node.to_string(),
        progress: ProgressEntry::empty(target, StreamId::new(stream_id), 0),
    }
}

fn leader_vote(term: u64) -> CommittedVoteOf<C> {
    Vote::<UTLeaderId>::new(term, 1).to_committed()
}

fn heartbeat(round: u64) -> HeartbeatEvent<C> {
    HeartbeatEvent {
        time: SendStamp::new(C::now(), round),
        matching: None,
        cluster_committed: None,
    }
}

/// Receives the next heartbeat request, and checks that it carries `vote` and nothing else.
async fn next_request(sent_rx: &mut MpscReceiverOf<C, Sent>, vote: &CommittedVoteOf<C>) -> String {
    let (node, request) = C::timeout(TEST_TIMEOUT, sent_rx.recv()).await.unwrap().unwrap();
    let expected: VoteOf<C> = vote.clone().into_vote();
    assert_eq!(expected, request.vote);
    assert_eq!(None, request.prev_log_id);
    assert_eq!(None, request.leader_commit);
    assert!(request.entries.is_empty());
    node
}

/// Receives the next notification, which must be the `HeartbeatProgress` of a heartbeat.
async fn next_progress(
    rx_notification: &mut MpscReceiverOf<C, Notification<C>>,
) -> (StreamId, SendStamp<InstantOf<C>>, u64) {
    let received = C::timeout(TEST_TIMEOUT, rx_notification.recv()).await.unwrap();
    match received {
        Some(Notification::HeartbeatProgress {
            stream_id,
            sending_time,
            target,
        }) => (stream_id, sending_time, target),
        Some(other) => panic!("expected HeartbeatProgress, got: {other}"),
        None => panic!("the notification channel is closed"),
    }
}

/// A worker whose node changed is replaced, even while its RPC is pending: the next heartbeat
/// goes through a client created from the new node. The old worker drops its pending RPC once
/// `election_timeout_max` has passed. A worker whose node did not change keeps its client.
#[test]
fn test_changed_node_replaces_the_worker_and_its_client() {
    C::run(async {
        let config = Arc::new(Config {
            heartbeat_interval: HEARTBEAT_MS,
            election_timeout_max: STOP_LIMIT_MS,
            ..Config::default()
        });
        let mut handle = HeartbeatWorkersHandle::<C>::new(1, config);
        let (mut factory, mut sent_rx) = RecordingFactory::new();
        factory.blocking.lock().unwrap().insert("old".to_string());
        let (tx_notification, mut rx_notification) = C::mpsc(16);
        let vote = leader_vote(1);

        tracing::info!("--- spawn workers for target 2 at `old` and target 3 at `three`");
        {
            let progresses = [progress(2, "old", 2), progress(3, "three", 3)];
            handle.spawn_workers(vote.clone(), &mut factory, &tx_notification, &progresses, false).await;
            let expected = vec![(2, "old".to_string()), (3, "three".to_string())];
            assert_eq!(expected, factory.created());
        }

        tracing::info!("--- a heartbeat to target 2 stays pending at `old`");
        {
            handle.broadcast([(2, heartbeat(1))]);
            let node = next_request(&mut sent_rx, &vote).await;
            assert_eq!("old", node);
        }

        tracing::info!("--- moving target 2 to `new` stops the pending worker and creates a client");
        {
            let progresses = [progress(2, "new", 2), progress(3, "three", 3)];
            handle.spawn_workers(vote.clone(), &mut factory, &tx_notification, &progresses, false).await;

            let dropped = C::timeout(TEST_TIMEOUT, factory.dropped(0)).await.unwrap();
            assert!(dropped.is_ok(), "the worker of the old node must stop");

            let expected = vec![(2, "old".to_string()), (3, "three".to_string()), (2, "new".to_string())];
            assert_eq!(expected, factory.created());
            let unchanged_dropped = factory.dropped(1).now_or_never();
            assert!(unchanged_dropped.is_none(), "the worker of target 3 must be kept");
        }

        tracing::info!("--- the next heartbeat to target 2 goes through `new`");
        {
            let event = heartbeat(2);
            handle.broadcast([(2, event.clone())]);
            let node = next_request(&mut sent_rx, &vote).await;
            assert_eq!("new", node);
            let progress = next_progress(&mut rx_notification).await;
            assert_eq!((StreamId::new(2), event.time, 2), progress);
        }

        tracing::info!("--- target 3 still uses the client it started with");
        {
            let event = heartbeat(3);
            handle.broadcast([(3, event.clone())]);
            let node = next_request(&mut sent_rx, &vote).await;
            assert_eq!("three", node);
            let progress = next_progress(&mut rx_notification).await;
            assert_eq!((StreamId::new(3), event.time, 3), progress);
            assert_eq!(3, factory.created().len(), "no client is created for an unchanged node");
        }
    });
}

/// Removing a target stops its worker; adding it back creates a new client, even for the same
/// node.
#[test]
fn test_removed_target_is_stopped_and_readded_with_a_new_client() {
    C::run(async {
        let config = Arc::new(Config {
            heartbeat_interval: HEARTBEAT_MS,
            ..Config::default()
        });
        let mut handle = HeartbeatWorkersHandle::<C>::new(1, config);
        let (mut factory, mut sent_rx) = RecordingFactory::new();
        let (tx_notification, mut rx_notification) = C::mpsc(16);
        let vote = leader_vote(1);

        tracing::info!("--- spawn a worker for target 2, then spawn none");
        {
            let progresses = [progress(2, "two", 2)];
            handle.spawn_workers(vote.clone(), &mut factory, &tx_notification, &progresses, false).await;
            handle.spawn_workers(vote.clone(), &mut factory, &tx_notification, [], false).await;

            let dropped = C::timeout(TEST_TIMEOUT, factory.dropped(0)).await.unwrap();
            assert!(dropped.is_ok(), "the worker of a removed target must stop");
            assert!(handle.workers.is_empty());
        }

        tracing::info!("--- adding target 2 back creates a new client for the same node");
        {
            let progresses = [progress(2, "two", 4)];
            handle.spawn_workers(vote.clone(), &mut factory, &tx_notification, &progresses, false).await;
            let expected = vec![(2, "two".to_string()), (2, "two".to_string())];
            assert_eq!(expected, factory.created());

            let event = heartbeat(1);
            handle.broadcast([(2, event.clone())]);
            let node = next_request(&mut sent_rx, &vote).await;
            assert_eq!("two", node);
            let progress = next_progress(&mut rx_notification).await;
            assert_eq!((StreamId::new(4), event.time, 2), progress);
        }
    });
}

/// `close_old` replaces every worker, even for an unchanged node, so heartbeats carry the new
/// leader vote and stream id.
#[test]
fn test_close_old_replaces_workers_for_a_new_leader_vote() {
    C::run(async {
        let config = Arc::new(Config {
            heartbeat_interval: HEARTBEAT_MS,
            ..Config::default()
        });
        let mut handle = HeartbeatWorkersHandle::<C>::new(1, config);
        let (mut factory, mut sent_rx) = RecordingFactory::new();
        let (tx_notification, mut rx_notification) = C::mpsc(16);

        tracing::info!("--- a worker for target 2 sends heartbeats with the vote of term 1");
        {
            let vote = leader_vote(1);
            let progresses = [progress(2, "two", 2)];
            handle.spawn_workers(vote.clone(), &mut factory, &tx_notification, &progresses, false).await;
            let event = heartbeat(1);
            handle.broadcast([(2, event.clone())]);
            let node = next_request(&mut sent_rx, &vote).await;
            assert_eq!("two", node);
            let progress = next_progress(&mut rx_notification).await;
            assert_eq!((StreamId::new(2), event.time, 2), progress);
        }

        tracing::info!("--- with close_old, the leader of term 2 replaces the worker of the same node");
        {
            let vote = leader_vote(2);
            let progresses = [progress(2, "two", 5)];
            handle.spawn_workers(vote.clone(), &mut factory, &tx_notification, &progresses, true).await;

            let dropped = C::timeout(TEST_TIMEOUT, factory.dropped(0)).await.unwrap();
            assert!(dropped.is_ok(), "close_old must stop the old worker");
            let expected = vec![(2, "two".to_string()), (2, "two".to_string())];
            assert_eq!(expected, factory.created());

            let event = heartbeat(2);
            handle.broadcast([(2, event.clone())]);
            let node = next_request(&mut sent_rx, &vote).await;
            assert_eq!("two", node);
            let progress = next_progress(&mut rx_notification).await;
            assert_eq!((StreamId::new(5), event.time, 2), progress);
        }
    });
}
