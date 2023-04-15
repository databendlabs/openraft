//! State machine worker and its supporting types.
//!
//! This worker runs in a separate task and is the only one that can mutate the state machine.
//! It is responsible for applying log entries, building/receiving snapshot  and sending responses
//! to the RaftCore.

use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use tokio::sync::oneshot;

use crate::core::snapshot_state::SnapshotRequestId;
use crate::core::streaming_state::Streaming;
use crate::core::ApplyResult;
use crate::core::ApplyingEntry;
use crate::entry::RaftPayload;
use crate::error::InstallSnapshotError;
use crate::error::SnapshotMismatch;
use crate::raft::InstallSnapshotRequest;
use crate::raft::RaftMsg;
use crate::storage::RaftLogStorage;
use crate::storage::RaftStateMachine;
use crate::summary::MessageSummary;
use crate::RaftLogId;
use crate::RaftNetworkFactory;
use crate::RaftSnapshotBuilder;
use crate::RaftTypeConfig;
use crate::Snapshot;
use crate::SnapshotMeta;
use crate::SnapshotSegmentId;
use crate::StorageError;
use crate::StorageIOError;

pub(crate) mod command;
pub(crate) mod response;

pub(crate) use command::Command;
pub(crate) use command::CommandPayload;
#[allow(unused_imports)] pub(crate) use command::CommandSeq;
pub(crate) use response::CommandResult;
pub(crate) use response::Response;

// TODO: replace it with Snapshot<NID,N,SD>
/// Received snapshot from the leader.
struct Received<C: RaftTypeConfig, SM: RaftStateMachine<C>> {
    snapshot_meta: SnapshotMeta<C::NodeId, C::Node>,
    data: Box<SM::SnapshotData>,
}

impl<C, SM> Received<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    pub(crate) fn new(snapshot_meta: SnapshotMeta<C::NodeId, C::Node>, data: Box<SM::SnapshotData>) -> Self {
        Self { snapshot_meta, data }
    }
}

/// State machine worker handle for sending command to it.
pub(crate) struct Handle<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    cmd_tx: mpsc::UnboundedSender<Command<C, SM>>,
    #[allow(dead_code)]
    join_handle: tokio::task::JoinHandle<()>,
}

impl<C, SM> Handle<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    pub(crate) fn send(&mut self, cmd: Command<C, SM>) -> Result<(), mpsc::error::SendError<Command<C, SM>>> {
        tracing::debug!("sending command to state machine worker: {:?}", cmd);
        self.cmd_tx.send(cmd)
    }
}

pub(crate) struct Worker<C, N, LS, SM>
where
    C: RaftTypeConfig,
    N: RaftNetworkFactory<C>,
    LS: RaftLogStorage<C>,
    SM: RaftStateMachine<C>,
{
    state_machine: SM,

    streaming: Option<Streaming<SM::SnapshotData>>,

    received: Option<Received<C, SM>>,

    cmd_rx: mpsc::UnboundedReceiver<Command<C, SM>>,

    resp_tx: mpsc::UnboundedSender<RaftMsg<C, N, LS>>,
}

impl<C, N, LS, SM> Worker<C, N, LS, SM>
where
    C: RaftTypeConfig,
    N: RaftNetworkFactory<C>,
    LS: RaftLogStorage<C>,
    SM: RaftStateMachine<C>,
{
    /// Spawn a new state machine worker, return a controlling handle.
    pub(crate) fn spawn(state_machine: SM, resp_tx: mpsc::UnboundedSender<RaftMsg<C, N, LS>>) -> Handle<C, SM> {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();

        let worker = Worker {
            state_machine,
            streaming: None,
            received: None,
            cmd_rx,
            resp_tx,
        };

        let join_handle = worker.do_spawn();

        Handle { cmd_tx, join_handle }
    }

    fn do_spawn(mut self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let res = self.worker_loop().await;

            if let Err(err) = res {
                tracing::error!("{} while execute state machine command", err,);

                let _ = self.resp_tx.send(RaftMsg::StateMachine {
                    command_result: CommandResult {
                        command_seq: 0,
                        result: Err(err),
                    },
                });
            }
        })
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn worker_loop(&mut self) -> Result<(), StorageError<C::NodeId>> {
        loop {
            let cmd = self.cmd_rx.recv().await;
            let cmd = match cmd {
                None => {
                    tracing::info!("{}: rx closed, state machine worker quit", func_name!());
                    return Ok(());
                }
                Some(x) => x,
            };

            tracing::debug!("{}: received command: {:?}", func_name!(), cmd);

            let command_result = match cmd.payload {
                CommandPayload::BuildSnapshot => {
                    let resp = self.build_snapshot().await?;
                    CommandResult::new(cmd.command_id, Ok(Response::BuildSnapshot(resp)))
                }
                CommandPayload::GetSnapshot { tx } => {
                    #[allow(clippy::let_unit_value)]
                    let resp = self.get_snapshot(tx).await?;
                    CommandResult::new(cmd.command_id, Ok(Response::GetSnapshot(resp)))
                }
                CommandPayload::ReceiveSnapshotChunk { req, tx } => {
                    #[allow(clippy::let_unit_value)]
                    let resp = self.receive_snapshot_chunk(req, tx).await?;
                    CommandResult::new(cmd.command_id, Ok(Response::ReceiveSnapshotChunk(resp)))
                }
                CommandPayload::FinalizeSnapshot { install, snapshot_meta } => {
                    let resp = if install {
                        let resp = self.install_snapshot(snapshot_meta).await?;
                        Some(resp)
                    } else {
                        self.received = None;
                        None
                    };
                    CommandResult::new(cmd.command_id, Ok(Response::InstallSnapshot(resp)))
                }
                CommandPayload::Apply { entries } => {
                    let resp = self.apply(entries).await?;
                    CommandResult::new(cmd.command_id, Ok(Response::Apply(resp)))
                }
            };

            let _ = self.resp_tx.send(RaftMsg::StateMachine { command_result });

            (cmd.respond)();
        }
    }
    #[tracing::instrument(level = "info", skip_all)]
    async fn apply(&mut self, entries: Vec<C::Entry>) -> Result<ApplyResult<C>, StorageError<C::NodeId>> {
        // TODO: prepare response before apply_to_state_machine,
        //       so that an Entry does not need to be Clone,
        //       and no references will be used by apply_to_state_machine

        let since = entries.first().map(|x| x.get_log_id().index).unwrap();
        let end = entries.last().map(|x| x.get_log_id().index + 1).unwrap();
        let last_applied = entries.last().map(|x| *x.get_log_id()).unwrap();

        // Fake complain: avoid using `collect()` when not needed
        #[allow(clippy::needless_collect)]
        let applying_entries = entries
            .iter()
            .map(|e| ApplyingEntry::new(*e.get_log_id(), e.get_membership().cloned()))
            .collect::<Vec<_>>();

        let apply_results = self.state_machine.apply(entries).await?;

        let resp = ApplyResult {
            since,
            end,
            last_applied,
            applying_entries,
            apply_results,
        };

        Ok(resp)
    }

    #[tracing::instrument(level = "info", skip_all)]
    async fn build_snapshot(&mut self) -> Result<SnapshotMeta<C::NodeId, C::Node>, StorageError<C::NodeId>> {
        // TODO: move it to another task
        // use futures::future::abortable;
        // let (fu, abort_handle) = abortable(async move { builder.build_snapshot().await });

        tracing::info!("{}", func_name!());

        let mut builder = self.state_machine.get_snapshot_builder().await;
        let snapshot = builder.build_snapshot().await?;

        Ok(snapshot.meta)
    }

    #[tracing::instrument(level = "info", skip_all)]
    async fn get_snapshot(
        &mut self,
        tx: oneshot::Sender<Option<Snapshot<C::NodeId, C::Node, SM::SnapshotData>>>,
    ) -> Result<(), StorageError<C::NodeId>> {
        tracing::info!("{}", func_name!());

        let snapshot = self.state_machine.get_current_snapshot().await?;

        tracing::info!("sending back snapshot");
        let _ = tx.send(snapshot);
        Ok(())
    }

    #[tracing::instrument(level = "info", skip_all)]
    async fn receive_snapshot_chunk(
        &mut self,
        req: InstallSnapshotRequest<C>,
        tx: oneshot::Sender<Result<(), InstallSnapshotError>>,
    ) -> Result<(), StorageError<C::NodeId>> {
        let snapshot_meta = req.meta.clone();
        let done = req.done;
        let offset = req.offset;

        let req_id = SnapshotRequestId::new(*req.vote.leader_id(), snapshot_meta.snapshot_id.clone(), offset);

        tracing::info!(
            req = display(req.summary()),
            snapshot_req_id = debug(&req_id),
            "{}",
            func_name!()
        );

        let curr_id = self.streaming.as_ref().map(|s| &s.snapshot_id);

        // Changed to another stream. re-init snapshot state.
        if curr_id != Some(&req.meta.snapshot_id) {
            if req.offset > 0 {
                let mismatch = SnapshotMismatch {
                    expect: SnapshotSegmentId {
                        id: snapshot_meta.snapshot_id.clone(),
                        offset: 0,
                    },
                    got: SnapshotSegmentId {
                        id: snapshot_meta.snapshot_id.clone(),
                        offset,
                    },
                };

                tracing::warn!("snapshot mismatch: {}", mismatch);
                let _ = tx.send(Err(mismatch.into()));
                return Ok(());
            }

            let snapshot_data = self.state_machine.begin_receiving_snapshot().await?;
            self.streaming = Some(Streaming::new(req.meta.snapshot_id.clone(), snapshot_data));
        }

        let streaming = self.streaming.as_mut().unwrap();
        streaming.receive(req).await?;

        tracing::info!(snapshot_req_id = debug(&req_id), "received snapshot chunk");

        if done {
            let streaming = self.streaming.take().unwrap();
            let mut data = streaming.snapshot_data;

            data.as_mut()
                .shutdown()
                .await
                .map_err(|e| StorageIOError::write_snapshot(Some(snapshot_meta.signature()), &e))?;

            tracing::info!("store completed streaming snapshot: {:?}", snapshot_meta);
            self.received = Some(Received::new(snapshot_meta, data));
        }

        let _ = tx.send(Ok(()));

        Ok(())
    }

    #[tracing::instrument(level = "info", skip_all)]
    async fn install_snapshot(
        &mut self,
        snapshot_meta: SnapshotMeta<C::NodeId, C::Node>,
    ) -> Result<SnapshotMeta<C::NodeId, C::Node>, StorageError<C::NodeId>> {
        tracing::info!("installing snapshot: {:?}", snapshot_meta);

        // If there is no received data, it is a bug
        let received = self.received.take().unwrap();

        debug_assert!(
            received.snapshot_meta == snapshot_meta,
            "expected snapshot meta: {:?}, got: {:?}",
            received.snapshot_meta,
            snapshot_meta
        );

        self.state_machine.install_snapshot(&snapshot_meta, received.data).await?;

        tracing::info!("Done install_snapshot, meta: {:?}", snapshot_meta);

        Ok(snapshot_meta)
    }
}
