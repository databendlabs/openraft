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
use crate::raft::InstallSnapshotRequest;
use crate::storage::RaftStateMachine;
use crate::summary::MessageSummary;
use crate::AsyncRuntime;
use crate::RaftLogId;
use crate::RaftSnapshotBuilder;
use crate::RaftTypeConfig;
use crate::Snapshot;
use crate::SnapshotMeta;
use crate::StorageError;
use crate::StorageIOError;

pub(crate) mod command;
pub(crate) mod response;

pub(crate) use command::Command;
pub(crate) use command::CommandPayload;
#[allow(unused_imports)] pub(crate) use command::CommandSeq;
pub(crate) use response::CommandResult;
pub(crate) use response::Response;

use crate::core::notify::Notify;

/// State machine worker handle for sending command to it.
pub(crate) struct Handle<C>
where C: RaftTypeConfig
{
    cmd_tx: mpsc::UnboundedSender<Command<C>>,
    #[allow(dead_code)]
    join_handle: <C::AsyncRuntime as AsyncRuntime>::JoinHandle<()>,
}

impl<C> Handle<C>
where C: RaftTypeConfig
{
    pub(crate) fn send(&mut self, cmd: Command<C>) -> Result<(), mpsc::error::SendError<Command<C>>> {
        tracing::debug!("sending command to state machine worker: {:?}", cmd);
        self.cmd_tx.send(cmd)
    }
}

pub(crate) struct Worker<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    state_machine: SM,

    streaming: Option<Streaming<C>>,

    received: Option<Snapshot<C>>,

    cmd_rx: mpsc::UnboundedReceiver<Command<C>>,

    resp_tx: mpsc::UnboundedSender<Notify<C>>,
}

impl<C, SM> Worker<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    /// Spawn a new state machine worker, return a controlling handle.
    pub(crate) fn spawn(state_machine: SM, resp_tx: mpsc::UnboundedSender<Notify<C>>) -> Handle<C> {
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

    fn do_spawn(mut self) -> <C::AsyncRuntime as AsyncRuntime>::JoinHandle<()> {
        C::AsyncRuntime::spawn(async move {
            let res = self.worker_loop().await;

            if let Err(err) = res {
                tracing::error!("{} while execute state machine command", err,);

                let _ = self.resp_tx.send(Notify::StateMachine {
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

            match cmd.payload {
                CommandPayload::BuildSnapshot => {
                    tracing::info!("{}: build snapshot", func_name!());

                    // It is a read operation and is spawned, and it responds in another task
                    self.build_snapshot(cmd.seq, self.resp_tx.clone()).await;
                }
                CommandPayload::GetSnapshot { tx } => {
                    tracing::info!("{}: get snapshot", func_name!());

                    self.get_snapshot(tx).await?;
                    // GetSnapshot does not respond to RaftCore
                }
                CommandPayload::ReceiveSnapshotChunk { req } => {
                    tracing::info!(
                        req = display(req.summary()),
                        "{}: received snapshot chunk",
                        func_name!()
                    );

                    #[allow(clippy::let_unit_value)]
                    let resp = self.receive_snapshot_chunk(req).await?;
                    let res = CommandResult::new(cmd.seq, Ok(Response::ReceiveSnapshotChunk(resp)));
                    let _ = self.resp_tx.send(Notify::sm(res));
                }
                CommandPayload::FinalizeSnapshot { install, snapshot_meta } => {
                    tracing::info!(
                        install = display(install),
                        snapshot_meta = display(snapshot_meta.summary()),
                        "{}: finalize and install snapshot",
                        func_name!()
                    );

                    let resp = if install {
                        let resp = self.install_snapshot(snapshot_meta).await?;
                        Some(resp)
                    } else {
                        self.received = None;
                        None
                    };
                    let res = CommandResult::new(cmd.seq, Ok(Response::InstallSnapshot(resp)));
                    let _ = self.resp_tx.send(Notify::sm(res));
                }
                CommandPayload::Apply { entries } => {
                    let resp = self.apply(entries).await?;
                    let res = CommandResult::new(cmd.seq, Ok(Response::Apply(resp)));
                    let _ = self.resp_tx.send(Notify::sm(res));
                }
            };
        }
    }
    #[tracing::instrument(level = "debug", skip_all)]
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

        let n_entries = applying_entries.len();

        let apply_results = self.state_machine.apply(entries).await?;

        let n_replies = apply_results.len();

        debug_assert_eq!(
            n_entries, n_replies,
            "n_entries: {} should equal n_replies: {}",
            n_entries, n_replies
        );

        let resp = ApplyResult {
            since,
            end,
            last_applied,
            applying_entries,
            apply_results,
        };

        Ok(resp)
    }

    /// Build a snapshot from the state machine.
    ///
    /// Building snapshot is a read-only operation, so it can be run in another task in parallel.
    /// This parallelization depends on the [`RaftSnapshotBuilder`] implementation returned  by
    /// [`get_snapshot_builder()`](`RaftStateMachine::get_snapshot_builder()`): The builder must:
    /// - hold a consistent view of the state machine that won't be affected by further writes such
    ///   as applying a log entry,
    /// - or it must be able to acquire a lock that prevents any write operations.
    #[tracing::instrument(level = "info", skip_all)]
    async fn build_snapshot(&mut self, seq: CommandSeq, resp_tx: mpsc::UnboundedSender<Notify<C>>) {
        // TODO: need to be abortable?
        // use futures::future::abortable;
        // let (fu, abort_handle) = abortable(async move { builder.build_snapshot().await });

        tracing::info!("{}", func_name!());

        let mut builder = self.state_machine.get_snapshot_builder().await;

        let _handle = C::AsyncRuntime::spawn(async move {
            let res = builder.build_snapshot().await;
            let res = res.map(|snap| Response::BuildSnapshot(snap.meta));
            let cmd_res = CommandResult::new(seq, res);
            let _ = resp_tx.send(Notify::sm(cmd_res));
        });
        tracing::info!("{} returning; spawned building snapshot task", func_name!());
    }

    #[tracing::instrument(level = "info", skip_all)]
    async fn get_snapshot(&mut self, tx: oneshot::Sender<Option<Snapshot<C>>>) -> Result<(), StorageError<C::NodeId>> {
        tracing::info!("{}", func_name!());

        let snapshot = self.state_machine.get_current_snapshot().await?;

        tracing::info!(
            "sending back snapshot: meta: {:?}",
            snapshot.as_ref().map(|s| s.meta.summary())
        );
        let _ = tx.send(snapshot);
        Ok(())
    }

    #[tracing::instrument(level = "info", skip_all)]
    async fn receive_snapshot_chunk(&mut self, req: InstallSnapshotRequest<C>) -> Result<(), StorageError<C::NodeId>> {
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
            self.received = Some(Snapshot::new(snapshot_meta, data));
        }

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
            received.meta == snapshot_meta,
            "expected snapshot meta: {:?}, got: {:?}",
            received.meta,
            snapshot_meta
        );

        self.state_machine.install_snapshot(&snapshot_meta, received.snapshot).await?;

        tracing::info!("Done install_snapshot, meta: {:?}", snapshot_meta);

        Ok(snapshot_meta)
    }
}
