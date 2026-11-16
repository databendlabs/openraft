use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use super::SnapshotReader;
use crate::Config;
use crate::OptionalSend;
use crate::RaftNetworkFactory;
use crate::async_runtime::MpscReceiver;
use crate::async_runtime::MpscSender;
use crate::core::SharedReplicateBatch;
use crate::core::sm::Command;
use crate::engine::testing::UTConfig;
use crate::errors::RPCError;
use crate::errors::ReplicationClosed;
use crate::errors::StreamingError;
use crate::network::RPCOption;
use crate::network::RaftNetworkV2;
use crate::progress::inflight_id::InflightId;
use crate::progress::stream_id::StreamId;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendResult;
use crate::raft::SnapshotResponse;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::replication::replication_context::ReplicationContext;
use crate::replication::snapshot_transmitter::SnapshotTransmitter;
use crate::replication::snapshot_transmitter_handle::SnapshotTransmitterHandle;
use crate::type_config::OneshotSender;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::SnapshotOf;
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
