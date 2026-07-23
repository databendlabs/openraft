use std::future::Future;
use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use futures_util::Stream;

use crate::Config;
use crate::OptionalSend;
use crate::StorageError;
use crate::async_runtime::MpscReceiver;
use crate::async_runtime::MpscSender;
use crate::async_runtime::OneshotSender;
use crate::base::BoxFuture;
use crate::base::BoxStream;
use crate::core::SharedReplicateBatch;
use crate::core::notification::Notification;
use crate::core::sm;
use crate::core::sm::handle::SnapshotReader;
use crate::engine::testing::UTConfig;
use crate::errors::RPCError;
use crate::errors::ReplicationClosed;
use crate::errors::StreamingError;
use crate::errors::Unreachable;
use crate::network::Backoff;
use crate::network::NetAppend;
use crate::network::NetBackoff;
use crate::network::NetSnapshot;
use crate::network::NetStreamAppend;
use crate::network::NetTransferLeader;
use crate::network::NetVote;
use crate::network::RPCOption;
use crate::network::RaftNetworkFactory;
use crate::progress::inflight_id::InflightId;
use crate::progress::stream_id::StreamId;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponse;
use crate::raft::SnapshotResponse;
use crate::raft::StreamAppendResult;
use crate::raft::TransferLeaderRequest;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::replication::replication_context::ReplicationContext;
use crate::replication::snapshot_transmitter::SnapshotTransmitter;
use crate::storage::Snapshot;
use crate::storage::SnapshotMeta;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::JoinHandleOf;
use crate::type_config::alias::MpscSenderOf;
use crate::type_config::alias::SnapshotOf;
use crate::type_config::alias::VoteOf;
use crate::type_config::alias::WatchReceiverOf;
use crate::vote::committed::CommittedVote;

type C = UTConfig;

struct MockNetworkFactory;

impl RaftNetworkFactory<C> for MockNetworkFactory {
    type Network = MockNetwork;

    async fn new_client(&mut self, _target: u64, _node: &<C as crate::RaftTypeConfig>::Node) -> MockNetwork {
        MockNetwork::default()
    }
}

/// Controls what [`MockNetwork::full_snapshot`] returns, so tests can drive
/// `SnapshotTransmitter` through each exit path of `stream_snapshot`.
#[derive(Debug, Clone)]
enum SnapshotResp {
    /// Success — returns the caller's vote unchanged.
    Ok,
    /// Success — but returns a vote higher than the sender's, triggering the `HigherVote` path.
    HigherVote(VoteOf<C>),
    /// Returns a `StorageError`, triggering the `StorageError` exit path.
    StorageError,
    /// Returns `Unreachable`, triggering the backoff loop.
    Unreachable,
}

#[derive(Debug, Clone)]
struct MockNetwork {
    snapshot_resp: SnapshotResp,
}

impl Default for MockNetwork {
    fn default() -> Self {
        Self {
            snapshot_resp: SnapshotResp::Ok,
        }
    }
}

impl NetBackoff<C> for MockNetwork {
    fn backoff(&self) -> Option<Backoff> {
        None
    }
}

impl NetAppend<C> for MockNetwork {
    async fn append_entries(
        &mut self,
        _rpc: AppendEntriesRequest<C>,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse<C>, RPCError<C>> {
        unimplemented!()
    }
}

impl NetStreamAppend<C> for MockNetwork {
    fn stream_append<'s, S>(
        &'s mut self,
        _input: S,
        _option: RPCOption,
    ) -> BoxFuture<'s, Result<BoxStream<'s, Result<StreamAppendResult<C>, RPCError<C>>>, RPCError<C>>>
    where
        S: Stream<Item = AppendEntriesRequest<C>> + OptionalSend + Unpin + 'static,
    {
        unimplemented!()
    }
}

impl NetVote<C> for MockNetwork {
    async fn vote(&mut self, _rpc: VoteRequest<C>, _option: RPCOption) -> Result<VoteResponse<C>, RPCError<C>> {
        unimplemented!()
    }
}

impl NetSnapshot<C> for MockNetwork {
    async fn full_snapshot(
        &mut self,
        vote: VoteOf<C>,
        _snapshot: SnapshotOf<C>,
        _cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        _option: RPCOption,
    ) -> Result<SnapshotResponse<C>, StreamingError<C>> {
        match &self.snapshot_resp {
            SnapshotResp::Ok => Ok(SnapshotResponse::new(vote)),
            SnapshotResp::HigherVote(higher) => Ok(SnapshotResponse::new(*higher)),
            SnapshotResp::StorageError => Err(StreamingError::StorageError(StorageError::read_snapshot(
                None,
                C::err_from_string("test storage error"),
            ))),
            SnapshotResp::Unreachable => Err(StreamingError::Unreachable(Unreachable::from_string(
                "test unreachable",
            ))),
        }
    }
}

impl NetTransferLeader<C> for MockNetwork {
    async fn transfer_leader(&mut self, _req: TransferLeaderRequest<C>, _option: RPCOption) -> Result<(), RPCError<C>> {
        unimplemented!()
    }
}

fn build_context(
    tx_notify: MpscSenderOf<C, Notification<C>>,
    cancel_rx: WatchReceiverOf<C, ()>,
) -> ReplicationContext<C> {
    build_context_with(tx_notify, cancel_rx, Config::default().validate().unwrap())
}

fn build_context_with(
    tx_notify: MpscSenderOf<C, Notification<C>>,
    cancel_rx: WatchReceiverOf<C, ()>,
    config: Config,
) -> ReplicationContext<C> {
    ReplicationContext {
        id: 1,
        target: 2,
        leader_vote: CommittedVote::default(),
        stream_id: StreamId::new(1),
        config: Arc::new(config),
        tx_notify,
        cancel_rx,
        replicate_batch: SharedReplicateBatch::default(),
    }
}

fn empty_snapshot() -> SnapshotOf<C> {
    Snapshot::new(SnapshotMeta::default(), Cursor::new(vec![]))
}

fn spawn_snapshot_worker() -> (MpscSenderOf<C, sm::Command<C>>, JoinHandleOf<C, ()>) {
    let (tx, mut rx) = C::mpsc::<sm::Command<C>>(1);

    let handle = C::spawn(async move {
        while let Some(cmd) = rx.recv().await {
            if let sm::Command::GetSnapshot { tx } = cmd {
                tx.send(Some(empty_snapshot())).ok();
            }
        }
    });

    (tx, handle)
}

#[test]
fn test_snapshot_transmitted_on_success() {
    C::run(async {
        let (tx_notify, mut rx_notify) = C::mpsc::<Notification<C>>(10);
        let (cancel_tx, cancel_rx) = C::watch_channel(());
        let replication_context = build_context(tx_notify, cancel_rx);

        let (sm_tx, _sm_handle) = spawn_snapshot_worker();
        let snapshot_reader = SnapshotReader::new_test(sm_tx.downgrade());

        let handle = SnapshotTransmitter::<C, MockNetworkFactory, ()>::spawn(
            replication_context,
            MockNetwork::default(),
            snapshot_reader,
            InflightId::new(7),
            cancel_tx,
        );

        // The successful path first reports heartbeat and replication progress,
        // then reports that the snapshot transmission is complete.
        let notification = rx_notify.recv().await.expect("heartbeat progress notification");
        assert!(
            matches!(notification, Notification::HeartbeatProgress { target: 2, .. }),
            "expected HeartbeatProgress notification"
        );

        let notification = rx_notify.recv().await.expect("replication progress notification");
        assert!(
            matches!(
                notification,
                Notification::ReplicationProgress {
                    progress,
                    inflight_id: Some(id),
                } if progress.target == 2 && *id == 7
            ),
            "expected ReplicationProgress notification"
        );

        let notification = rx_notify.recv().await.expect("snapshot transmitted notification");
        assert!(
            matches!(
                notification,
                Notification::SnapshotTransmitted {
                    target: 2,
                    inflight_id,
                } if *inflight_id == 7
            ),
            "expected SnapshotTransmitted notification"
        );

        // Ensure the transmitter task finishes cleanly.
        handle._join_handle.await.ok();
    });
}

#[test]
fn test_snapshot_transmitted_when_reader_closed() {
    C::run(async {
        let (tx_notify, mut rx_notify) = C::mpsc::<Notification<C>>(10);
        let (cancel_tx, cancel_rx) = C::watch_channel(());
        let replication_context = build_context(tx_notify, cancel_rx);

        // Create a weak sender with no live strong sender so `SnapshotReader::get_snapshot` fails
        // immediately and the transmitter exits through the `ReplicationError::Closed` path.
        let (sm_tx, _sm_rx) = C::mpsc::<sm::Command<C>>(1);
        let snapshot_reader = SnapshotReader::new_test(sm_tx.downgrade());
        drop(sm_tx);

        let handle = SnapshotTransmitter::<C, MockNetworkFactory, ()>::spawn(
            replication_context,
            MockNetwork::default(),
            snapshot_reader,
            InflightId::new(8),
            cancel_tx,
        );

        let notification = rx_notify.recv().await.expect("notification received");
        assert!(
            matches!(
                notification,
                Notification::SnapshotTransmitted {
                    target: 2,
                    inflight_id,
                } if *inflight_id == 8
            ),
            "expected SnapshotTransmitted notification"
        );

        handle._join_handle.await.ok();
    });
}

#[test]
fn test_snapshot_transmitted_on_higher_vote() {
    C::run(async {
        let (tx_notify, mut rx_notify) = C::mpsc::<Notification<C>>(10);
        let (cancel_tx, cancel_rx) = C::watch_channel(());
        let replication_context = build_context(tx_notify, cancel_rx);

        let (sm_tx, _sm_handle) = spawn_snapshot_worker();
        let snapshot_reader = SnapshotReader::new_test(sm_tx.downgrade());

        // The sender's vote defaults to (term=0, node=0). A response with a higher
        // vote triggers the `HigherVote` exit path in `send_snapshot`.
        let higher_vote = VoteOf::<C>::new(1, 1);
        let network = MockNetwork {
            snapshot_resp: SnapshotResp::HigherVote(higher_vote),
        };

        let handle = SnapshotTransmitter::<C, MockNetworkFactory, ()>::spawn(
            replication_context,
            network,
            snapshot_reader,
            InflightId::new(9),
            cancel_tx,
        );

        // HigherVote path: a HigherVote notification followed by SnapshotTransmitted.
        let notification = rx_notify.recv().await.expect("higher vote notification");
        assert!(
            matches!(notification, Notification::HigherVote { target: 2, .. }),
            "expected HigherVote notification"
        );

        let notification = rx_notify.recv().await.expect("snapshot transmitted notification");
        assert!(
            matches!(
                notification,
                Notification::SnapshotTransmitted {
                    target: 2,
                    inflight_id,
                } if *inflight_id == 9
            ),
            "expected SnapshotTransmitted notification after HigherVote"
        );

        handle._join_handle.await.ok();
    });
}

#[test]
fn test_snapshot_transmitted_on_storage_error() {
    C::run(async {
        let (tx_notify, mut rx_notify) = C::mpsc::<Notification<C>>(10);
        let (cancel_tx, cancel_rx) = C::watch_channel(());
        let replication_context = build_context(tx_notify, cancel_rx);

        let (sm_tx, _sm_handle) = spawn_snapshot_worker();
        let snapshot_reader = SnapshotReader::new_test(sm_tx.downgrade());

        let network = MockNetwork {
            snapshot_resp: SnapshotResp::StorageError,
        };

        let handle = SnapshotTransmitter::<C, MockNetworkFactory, ()>::spawn(
            replication_context,
            network,
            snapshot_reader,
            InflightId::new(10),
            cancel_tx,
        );

        // StorageError path: a StorageError notification followed by SnapshotTransmitted.
        let notification = rx_notify.recv().await.expect("storage error notification");
        assert!(
            matches!(notification, Notification::StorageError { .. }),
            "expected StorageError notification"
        );

        let notification = rx_notify.recv().await.expect("snapshot transmitted notification");
        assert!(
            matches!(
                notification,
                Notification::SnapshotTransmitted {
                    target: 2,
                    inflight_id,
                } if *inflight_id == 10
            ),
            "expected SnapshotTransmitted notification after StorageError"
        );

        handle._join_handle.await.ok();
    });
}

#[test]
fn test_snapshot_transmitted_on_backoff_cancel() {
    C::run(async {
        let (tx_notify, mut rx_notify) = C::mpsc::<Notification<C>>(10);
        let (cancel_tx, cancel_rx) = C::watch_channel(());

        // Use a long backoff so the task stays in the backoff sleep until we cancel it.
        let mut config = Config::default().validate().unwrap();
        config.backoff = "10s".to_string();
        let replication_context = build_context_with(tx_notify, cancel_rx, config);

        let (sm_tx, _sm_handle) = spawn_snapshot_worker();
        let snapshot_reader = SnapshotReader::new_test(sm_tx.downgrade());

        let network = MockNetwork {
            snapshot_resp: SnapshotResp::Unreachable,
        };

        let handle = SnapshotTransmitter::<C, MockNetworkFactory, ()>::spawn(
            replication_context,
            network,
            snapshot_reader,
            InflightId::new(11),
            cancel_tx,
        );

        // Give the task time to hit Unreachable and enter the backoff sleep.
        C::sleep(Duration::from_millis(200)).await;

        // Dropping the handle drops `_tx_cancel`, which makes `cancel_rx.changed()` return
        // immediately and the task exits through the backoff-cancel path.
        drop(handle);

        let notification = rx_notify.recv().await.expect("snapshot transmitted notification");
        assert!(
            matches!(
                notification,
                Notification::SnapshotTransmitted {
                    target: 2,
                    inflight_id,
                } if *inflight_id == 11
            ),
            "expected SnapshotTransmitted notification after backoff cancel"
        );
    });
}
