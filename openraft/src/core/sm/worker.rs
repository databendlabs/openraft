use tokio::sync::mpsc;

use crate::async_runtime::AsyncOneshotSendExt;
use crate::core::notify::Notify;
use crate::core::raft_msg::ResultSender;
use crate::core::sm::handle::Handle;
use crate::core::sm::Command;
use crate::core::sm::CommandPayload;
use crate::core::sm::CommandResult;
use crate::core::sm::CommandSeq;
use crate::core::sm::Response;
use crate::core::ApplyResult;
use crate::core::ApplyingEntry;
use crate::display_ext::DisplayOptionExt;
use crate::entry::RaftPayload;
use crate::storage::RaftStateMachine;
use crate::type_config::alias::JoinHandleOf;
use crate::AsyncRuntime;
use crate::RaftLogId;
use crate::RaftSnapshotBuilder;
use crate::RaftTypeConfig;
use crate::Snapshot;
use crate::StorageError;

pub(crate) struct Worker<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    state_machine: SM,

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
            cmd_rx,
            resp_tx,
        };

        let join_handle = worker.do_spawn();

        Handle { cmd_tx, join_handle }
    }

    fn do_spawn(mut self) -> JoinHandleOf<C, ()> {
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
                CommandPayload::InstallFullSnapshot { snapshot } => {
                    tracing::info!("{}: install complete snapshot", func_name!());

                    let meta = snapshot.meta.clone();
                    self.state_machine.install_snapshot(&meta, snapshot.snapshot).await?;

                    tracing::info!("Done install complete snapshot, meta: {}", meta);

                    let res = CommandResult::new(cmd.seq, Ok(Response::InstallSnapshot(Some(meta))));
                    let _ = self.resp_tx.send(Notify::sm(res));
                }
                CommandPayload::BeginReceivingSnapshot { tx } => {
                    tracing::info!("{}: BeginReceivingSnapshot", func_name!());

                    let snapshot_data = self.state_machine.begin_receiving_snapshot().await?;

                    let _ = tx.send(Ok(snapshot_data));
                    // No response to RaftCore
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
        // TODO: prepare response before apply,
        //       so that an Entry does not need to be Clone,
        //       and no references will be used by apply

        let since = entries.first().map(|x| x.get_log_id().index).unwrap();
        let end = entries.last().map(|x| x.get_log_id().index + 1).unwrap();
        let last_applied = entries.last().map(|x| x.get_log_id().clone()).unwrap();

        // Fake complain: avoid using `collect()` when not needed
        #[allow(clippy::needless_collect)]
        let applying_entries = entries
            .iter()
            .map(|e| ApplyingEntry::new(e.get_log_id().clone(), e.get_membership().cloned()))
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
    async fn get_snapshot(&mut self, tx: ResultSender<C, Option<Snapshot<C>>>) -> Result<(), StorageError<C::NodeId>> {
        tracing::info!("{}", func_name!());

        let snapshot = self.state_machine.get_current_snapshot().await?;

        tracing::info!(
            "sending back snapshot: meta: {}",
            snapshot.as_ref().map(|s| &s.meta).display()
        );
        let _ = tx.send(Ok(snapshot));
        Ok(())
    }
}
