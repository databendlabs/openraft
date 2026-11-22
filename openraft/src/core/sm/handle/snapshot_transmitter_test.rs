use std::collections::VecDeque;
use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use display_more::DisplayOptionExt;

use super::SnapshotReader;
use crate::Config;
use crate::OptionalSend;
use crate::RaftNetworkFactory;
use crate::async_runtime::MpscReceiver;
use crate::async_runtime::MpscSender;
use crate::core::SharedReplicateBatch;
use crate::core::notification::Notification;
use crate::core::sm::Command;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::errors::NetworkError;
use crate::errors::RPCError;
use crate::errors::ReplicationClosed;
use crate::errors::StreamingError;
use crate::errors::Timeout;
use crate::network::RPCOption;
use crate::network::RPCTypes;
use crate::network::RaftNetworkV2;
use crate::progress::inflight_id::InflightId;
use crate::progress::stream_id::StreamId;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendResult;
use crate::raft::SnapshotResponse;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::replication::replication_context::ReplicationContext;
use crate::replication::response::ReplicationResult;
use crate::replication::snapshot_transmitter::SnapshotTransmitter;
use crate::replication::snapshot_transmitter_handle::SnapshotTransmitterHandle;
use crate::type_config::OneshotSender;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::MpscReceiverOf;
use crate::type_config::alias::OneshotSenderOf;
use crate::type_config::alias::SmSnapshotOf;
use crate::type_config::alias::SnapshotMetaOf;
use crate::type_config::alias::SnapshotOf;
use crate::type_config::alias::StoredMembershipOf;
use crate::type_config::alias::VoteOf;
use crate::vote::raft_vote::RaftVoteExt;

type C = UTConfig;

const TEST_TIMEOUT: Duration = Duration::from_secs(1);

struct UnusedNetwork;

impl RaftNetworkFactory<C> for UnusedNetwork {
    type Network = Self;

    async fn new_client(&mut self, _target: u64, _node: &()) -> Self {
        unreachable!("snapshot acquisition must be canceled before networking")
    }
}

impl RaftNetworkV2<C> for UnusedNetwork {
    type SnapshotData = ();

    async fn append_entries(
        &mut self,
        _rpc: AppendEntriesRequest<C>,
        _option: RPCOption,
    ) -> Result<AppendResult<C>, RPCError<C>> {
        unreachable!("snapshot acquisition must be canceled before networking")
    }

    async fn vote(&mut self, _rpc: VoteRequest<C>, _option: RPCOption) -> Result<VoteResponse<C>, RPCError<C>> {
        unreachable!("snapshot acquisition must be canceled before networking")
    }

    async fn full_snapshot(
        &mut self,
        _vote: VoteOf<C>,
        _snapshot: SnapshotOf<C, ()>,
        _cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        _option: RPCOption,
    ) -> Result<SnapshotResponse<C>, StreamingError<C>> {
        unreachable!("snapshot acquisition must be canceled before networking")
    }
}

#[test]
fn cancel_snapshot_acquisition_before_delivery_or_response() {
    C::run(async {
        for deliver_request in [false, true] {
            tracing::info!(
                deliver_request,
                "cancel acquisition while the state machine cannot return a snapshot"
            );
            let (cmd_tx, mut cmd_rx) = C::mpsc(1);
            if !deliver_request {
                cmd_tx.send(Command::BuildSnapshot).await.unwrap();
            }
            let snapshot_reader = SnapshotReader {
                cmd_tx: cmd_tx.downgrade(),
            };
            let (cancel_tx, cancel_rx) = C::watch_channel(());
            let (_probe_tx, quorum_loss_probe_rx) = C::watch_channel(None);
            let (tx_notify, mut rx_notify) = C::mpsc(1);
            let vote = VoteOf::<C>::new(1, 0);
            let context = ReplicationContext {
                id: 0,
                target: 1,
                leader_vote: vote.to_committed(),
                stream_id: StreamId::new(1),
                config: Arc::new(Config::default()),
                tx_notify,
                cancel_rx,
                replicate_batch: SharedReplicateBatch::new(),
                quorum_loss_probe_rx,
            };
            let handle = SnapshotTransmitter::<C, UnusedNetwork>::spawn(
                context,
                UnusedNetwork,
                snapshot_reader,
                InflightId::new(1),
                cancel_tx,
            );
            let response_tx = if deliver_request {
                let command = C::timeout(TEST_TIMEOUT, cmd_rx.recv()).await.unwrap();
                let Some(Command::GetSnapshot { tx }) = command else {
                    panic!("expected a snapshot acquisition command");
                };
                Some(tx)
            } else {
                None
            };

            let SnapshotTransmitterHandle {
                _join_handle: join,
                _tx_cancel: cancel,
            } = handle;
            drop(cancel);
            let joined = C::timeout(TEST_TIMEOUT, join).await.unwrap();
            joined.unwrap();
            let notification = rx_notify.recv().await;
            let closed = notification.is_none();
            assert!(
                closed,
                "cancellation must release the transmitter without reporting a storage failure"
            );

            if let Some(tx) = response_tx {
                let response = tx.send(None);
                let waiter_dropped = response.is_err();
                assert!(waiter_dropped, "the accepted command must outlive its canceled waiter");
            } else {
                let command = cmd_rx.recv().await;
                assert_eq!(Some(Command::BuildSnapshot), command);
            }
            drop(cmd_tx);
            let command = cmd_rx.recv().await;
            assert_eq!(None, command);
        }
    });
}

/// A network whose snapshot transfers return the scripted results in order. It records the last
/// log id of each snapshot it is handed.
struct ScriptedNetwork {
    results: VecDeque<Result<SnapshotResponse<C>, StreamingError<C>>>,
    sent: Arc<Mutex<Vec<Option<LogIdOf<C>>>>>,
}

impl RaftNetworkFactory<C> for ScriptedNetwork {
    type Network = Self;

    async fn new_client(&mut self, _target: u64, _node: &()) -> Self {
        unreachable!("the test hands the network to the transmitter")
    }
}

impl RaftNetworkV2<C> for ScriptedNetwork {
    type SnapshotData = ();

    async fn append_entries(
        &mut self,
        _rpc: AppendEntriesRequest<C>,
        _option: RPCOption,
    ) -> Result<AppendResult<C>, RPCError<C>> {
        unreachable!("a snapshot transmitter sends no logs")
    }

    async fn vote(&mut self, _rpc: VoteRequest<C>, _option: RPCOption) -> Result<VoteResponse<C>, RPCError<C>> {
        unreachable!("a snapshot transmitter sends no votes")
    }

    async fn full_snapshot(
        &mut self,
        _vote: VoteOf<C>,
        snapshot: SnapshotOf<C, ()>,
        _cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        _option: RPCOption,
    ) -> Result<SnapshotResponse<C>, StreamingError<C>> {
        self.sent.lock().unwrap().push(snapshot.meta.last_log_id);
        self.results.pop_front().expect("every transfer has a scripted result")
    }
}

/// The vote of the leader that sends the snapshot: node 0 in term 1.
fn leader_vote() -> VoteOf<C> {
    VoteOf::<C>::new_committed(1, 0)
}

/// A snapshot of the logs through `index`.
fn snapshot(index: u64) -> SmSnapshotOf<C, ()> {
    SmSnapshotOf::<C, ()> {
        meta: SnapshotMetaOf::<C> {
            last_log_id: Some(log_id(1, 0, index)),
            last_membership: StoredMembershipOf::<C>::default(),
        },
        snapshot: (),
    }
}

/// The channels a test drives and watches a snapshot transmitter through.
struct Harness {
    cmd_rx: MpscReceiverOf<C, Command<C>>,
    rx_notify: MpscReceiverOf<C, Notification<C>>,
    sent: Arc<Mutex<Vec<Option<LogIdOf<C>>>>>,
}

impl Harness {
    /// Wait for the transmitter to ask the state machine for the snapshot to send.
    async fn snapshot_request(&mut self) -> OneshotSenderOf<C, Option<SmSnapshotOf<C, ()>>> {
        let command = C::timeout(TEST_TIMEOUT, self.cmd_rx.recv()).await.unwrap();
        let Some(Command::GetSnapshot { tx }) = command else {
            panic!("expect a snapshot acquisition command");
        };
        tx
    }

    /// Wait for the next notification to RaftCore, or `None` once the transmitter has exited.
    async fn next_notification(&mut self) -> Option<Notification<C>> {
        C::timeout(TEST_TIMEOUT, self.rx_notify.recv()).await.unwrap()
    }

    /// The last log id of each snapshot that the transmitter handed to the network, in order.
    fn sent(&self) -> Vec<Option<LogIdOf<C>>> {
        self.sent.lock().unwrap().clone()
    }
}

/// Spawn the transmitter of inflight operation 7, which sends a snapshot from the leader of
/// [`leader_vote()`] to node 1 over a network whose transfers return `results` in order. Next to
/// the harness, it returns a value that keeps the other channels of the transmitter open.
fn spawn_transmitter(
    results: impl IntoIterator<Item = Result<SnapshotResponse<C>, StreamingError<C>>>,
) -> (Harness, impl Sized) {
    let (cmd_tx, cmd_rx) = C::mpsc(1);
    let snapshot_reader = SnapshotReader {
        cmd_tx: cmd_tx.downgrade(),
    };
    let (cancel_tx, cancel_rx) = C::watch_channel(());
    let (probe_tx, quorum_loss_probe_rx) = C::watch_channel(None);
    let (tx_notify, rx_notify) = C::mpsc(8);
    let context = ReplicationContext {
        id: 0,
        target: 1,
        leader_vote: leader_vote().to_committed(),
        stream_id: StreamId::new(1),
        config: Arc::new(Config::default()),
        tx_notify,
        cancel_rx,
        replicate_batch: SharedReplicateBatch::new(),
        quorum_loss_probe_rx,
    };
    let sent = Arc::new(Mutex::new(vec![]));
    let network = ScriptedNetwork {
        results: results.into_iter().collect(),
        sent: sent.clone(),
    };
    let handle = SnapshotTransmitter::<C, ScriptedNetwork>::spawn(
        context,
        network,
        snapshot_reader,
        InflightId::new(7),
        cancel_tx,
    );

    let harness = Harness {
        cmd_rx,
        rx_notify,
        sent,
    };
    (harness, (cmd_tx, handle, probe_tx))
}

/// Unwrap `notification`, which must be the progress that credits node 1 with a matching log
/// id, into that log id and the inflight id it answers.
fn expect_matching(notification: Option<Notification<C>>) -> (Option<LogIdOf<C>>, Option<InflightId>) {
    let Some(Notification::ReplicationProgress {
        progress, inflight_id, ..
    }) = notification
    else {
        panic!("expect a replication progress, got: {}", notification.display());
    };
    assert_eq!(1, progress.target);
    let Ok(ReplicationResult(Ok(matching))) = progress.result else {
        panic!("expect a matching log id, got: {:?}", progress.result);
    };
    (matching, inflight_id)
}

/// A transfer that the target answers with success credits the target with the last log id of
/// the snapshot that the transmitter sent, for the transmitter's inflight id.
#[test]
fn successful_transfer_credits_the_sent_snapshot() {
    C::run(async {
        let (mut h, _channels) = spawn_transmitter([Ok(SnapshotResponse::new(leader_vote()))]);

        tracing::info!("--- the state machine hands over its snapshot through log 5, and node 1 installs it");
        {
            h.snapshot_request().await.send(Some(snapshot(5))).unwrap();
        }

        tracing::info!("--- the transmitter reports node 1 reachable, then credits it with log 5");
        {
            let notification = h.next_notification().await;
            let heartbeat = matches!(notification, Some(Notification::HeartbeatProgress { target: 1, .. }));
            assert!(
                heartbeat,
                "expect a heartbeat progress, got: {}",
                notification.display()
            );

            let (matching, inflight_id) = expect_matching(h.next_notification().await);
            assert_eq!(Some(log_id(1, 0, 5)), matching);
            assert_eq!(Some(InflightId::new(7)), inflight_id);
        }

        tracing::info!("--- the transmitter exits after one transfer");
        {
            let notification = h.next_notification().await;
            assert!(
                notification.is_none(),
                "expect no more notifications, got: {}",
                notification.display()
            );
            assert_eq!(vec![Some(log_id(1, 0, 5))], h.sent());
        }
    });
}

/// A transfer that fails credits nothing: the transmitter reports nothing for it and sends the
/// snapshot again. The state machine hands over a newer snapshot for each transfer, and the
/// transfer that succeeds credits the snapshot it sent.
#[test]
fn failed_transfer_credits_nothing() {
    C::run(async {
        let timeout = StreamingError::Timeout(Timeout {
            action: RPCTypes::InstallSnapshot,
            id: 0,
            target: 1,
            timeout: Duration::from_millis(10),
        });
        let network = StreamingError::Network(NetworkError::from_string("connection reset"));
        let succeed = Ok(SnapshotResponse::new(leader_vote()));
        let (mut h, _channels) = spawn_transmitter([Err(timeout), Err(network), succeed]);

        tracing::info!("--- the transfers of the snapshots through logs 5 and 6 fail");
        {
            h.snapshot_request().await.send(Some(snapshot(5))).unwrap();
            h.snapshot_request().await.send(Some(snapshot(6))).unwrap();
        }

        tracing::info!("--- the transmitter asks for a snapshot again, having reported neither failure");
        {
            let tx = h.snapshot_request().await;
            // It asks only after it has handled the second failure, so a report of either failure
            // would already be in the channel.
            let reported = h.rx_notify.try_recv();
            assert!(reported.is_err(), "expect no report of a failed transfer");
            tx.send(Some(snapshot(8))).unwrap();
        }

        tracing::info!("--- the transfer of the snapshot through log 8 succeeds and credits log 8");
        {
            let notification = h.next_notification().await;
            let heartbeat = matches!(notification, Some(Notification::HeartbeatProgress { target: 1, .. }));
            assert!(
                heartbeat,
                "expect a heartbeat progress, got: {}",
                notification.display()
            );

            let (matching, inflight_id) = expect_matching(h.next_notification().await);
            assert_eq!(Some(log_id(1, 0, 8)), matching);
            assert_eq!(Some(InflightId::new(7)), inflight_id);

            let notification = h.next_notification().await;
            assert!(
                notification.is_none(),
                "expect no more notifications, got: {}",
                notification.display()
            );
            let expected = vec![Some(log_id(1, 0, 5)), Some(log_id(1, 0, 6)), Some(log_id(1, 0, 8))];
            assert_eq!(expected, h.sent());
        }
    });
}

/// A target that answers with a vote greater than the leader's has rejected the snapshot: the
/// transmitter reports the higher vote and credits nothing.
#[test]
fn transfer_answered_with_a_higher_vote_credits_nothing() {
    C::run(async {
        let higher = VoteOf::<C>::new_committed(2, 1);
        let (mut h, _channels) = spawn_transmitter([Ok(SnapshotResponse::new(higher))]);

        tracing::info!("--- node 1 has voted for a newer leader, and rejects the snapshot through log 5");
        {
            h.snapshot_request().await.send(Some(snapshot(5))).unwrap();
        }

        tracing::info!("--- the transmitter reports only the higher vote, then exits");
        {
            let notification = h.next_notification().await;
            let Some(Notification::HigherVote {
                target, higher: seen, ..
            }) = notification
            else {
                panic!("expect a higher vote, got: {}", notification.display());
            };
            assert_eq!(1, target);
            assert_eq!(higher, seen);

            let notification = h.next_notification().await;
            assert!(
                notification.is_none(),
                "expect no progress, got: {}",
                notification.display()
            );
        }
    });
}
