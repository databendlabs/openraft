use std::collections::BTreeMap;

use anyerror::AnyError;
use tracing_futures::Instrument;

use crate::RaftLogReader;
use crate::RaftSnapshotBuilder;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::async_runtime::MpscUnboundedReceiver;
use crate::async_runtime::MpscUnboundedSender;
use crate::async_runtime::OneshotSender;
use crate::core::ApplyResult;
use crate::core::notification::Notification;
use crate::core::sm::Command;
use crate::core::sm::CommandResult;
use crate::core::sm::Response;
use crate::core::sm::handle::Handle;
use crate::display_ext::DisplayOptionExt;
use crate::display_ext::DisplaySliceExt;
use crate::entry::RaftEntry;
use crate::entry::RaftPayload;
use crate::raft::ClientWriteResponse;
use crate::raft::responder::Responder;
use crate::raft::responder::either::OneshotOrUserDefined;
#[cfg(doc)]
use crate::storage::RaftLogStorage;
use crate::storage::RaftStateMachine;
use crate::storage::Snapshot;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::JoinHandleOf;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::MpscUnboundedReceiverOf;
use crate::type_config::alias::MpscUnboundedSenderOf;
use crate::type_config::alias::OneshotSenderOf;

pub(crate) struct Worker<C, SM, LR>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
    LR: RaftLogReader<C>,
{
    /// The application state machine implementation.
    state_machine: SM,

    /// Read logs from the [`RaftLogStorage`] implementation to apply them to the state machine.
    log_reader: LR,

    /// Read command from RaftCore to execute.
    cmd_rx: MpscUnboundedReceiverOf<C, Command<C>>,

    /// Send back the result of the command to RaftCore.
    resp_tx: MpscUnboundedSenderOf<C, Notification<C>>,
}

impl<C, SM, LR> Worker<C, SM, LR>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
    LR: RaftLogReader<C>,
{
    /// Spawn a new state machine worker, return a controlling handle.
    pub(crate) fn spawn(
        state_machine: SM,
        log_reader: LR,
        resp_tx: MpscUnboundedSenderOf<C, Notification<C>>,
        span: tracing::Span,
    ) -> Handle<C> {
        let (cmd_tx, cmd_rx) = C::mpsc_unbounded();

        let worker = Worker {
            state_machine,
            log_reader,
            cmd_rx,
            resp_tx,
        };

        let join_handle = worker.do_spawn(span);

        Handle { cmd_tx, join_handle }
    }

    fn do_spawn(mut self, span: tracing::Span) -> JoinHandleOf<C, ()> {
        let fu = async move {
            let res = self.worker_loop().await;

            if let Err(err) = res {
                tracing::error!("{} while execute state machine command", err,);

                let _ = self.resp_tx.send(Notification::StateMachine {
                    command_result: CommandResult { result: Err(err) },
                });
            }
        };
        C::spawn(fu.instrument(span))
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn worker_loop(&mut self) -> Result<(), StorageError<C>> {
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

            match cmd {
                Command::BuildSnapshot => {
                    tracing::info!("{}: build snapshot", func_name!());

                    // It is a read operation and is spawned, and it responds in another task
                    self.build_snapshot(self.resp_tx.clone()).await;
                }
                Command::GetSnapshot { tx } => {
                    tracing::info!("{}: get snapshot", func_name!());

                    self.get_snapshot(tx).await?;
                    // GetSnapshot does not respond to RaftCore
                }
                Command::InstallFullSnapshot { io_id, snapshot } => {
                    tracing::info!("{}: install complete snapshot", func_name!());

                    let meta = snapshot.meta.clone();
                    self.state_machine.install_snapshot(&meta, snapshot.snapshot).await?;

                    tracing::info!("Done install complete snapshot, meta: {}", meta);

                    let res = CommandResult::new(Ok(Response::InstallSnapshot((io_id, Some(meta)))));
                    let _ = self.resp_tx.send(Notification::sm(res));
                }
                Command::BeginReceivingSnapshot { tx } => {
                    tracing::info!("{}: BeginReceivingSnapshot", func_name!());

                    let snapshot_data = self.state_machine.begin_receiving_snapshot().await?;

                    let _ = tx.send(snapshot_data);
                    // No response to RaftCore
                }
                Command::Apply {
                    first,
                    last,
                    mut client_resp_channels,
                } => {
                    let resp = self.apply(first, last, &mut client_resp_channels).await?;
                    let res = CommandResult::new(Ok(Response::Apply(resp)));
                    let _ = self.resp_tx.send(Notification::sm(res));
                }
                Command::Func { func, input_sm_type } => {
                    tracing::debug!("{}: run user defined Func", func_name!());

                    let maybe_future = func(&mut self.state_machine);
                    match maybe_future {
                        Some(future) => future.await,
                        None => {
                            tracing::warn!(
                                "User-defined SM function uses incorrect state machine type, expected: {}, got: {}",
                                std::any::type_name::<SM>(),
                                input_sm_type
                            );
                        }
                    };
                }
            };
        }
    }
    #[tracing::instrument(level = "debug", skip_all)]
    async fn apply(
        &mut self,
        first: LogIdOf<C>,
        last: LogIdOf<C>,
        client_resp_channels: &mut BTreeMap<u64, OneshotOrUserDefined<C>>,
    ) -> Result<ApplyResult<C>, StorageError<C>> {
        // TODO: prepare response before apply,
        //       so that an Entry does not need to be Clone,
        //       and no references will be used by apply

        let since = first.index();
        let end = last.index() + 1;

        let entries = self.log_reader.try_get_log_entries(since..end).await?;
        if entries.len() != (end - since) as usize {
            return Err(StorageError::read_logs(AnyError::error(format!(
                "returned log entries count({}) does not match the input([{}, {}]))",
                entries.len(),
                since,
                end
            ))));
        }
        tracing::debug!(entries = display(entries.display()), "about to apply");

        let last_applied = last;

        // Fake complain: avoid using `collect()` when not needed
        #[allow(clippy::needless_collect)]
        let applying_entries = entries.iter().map(|e| (e.log_id(), e.get_membership())).collect::<Vec<_>>();

        let n_entries = end - since;

        let apply_results = self.state_machine.apply(entries).await?;

        let n_replies = apply_results.len() as u64;

        debug_assert_eq!(
            n_entries, n_replies,
            "n_entries: {} should equal n_replies: {}",
            n_entries, n_replies
        );

        let mut results = apply_results.into_iter();
        let mut applying_entries = applying_entries.into_iter();
        for log_index in since..end {
            let (log_id, membership) = applying_entries.next().unwrap();
            let resp = results.next().unwrap();
            let tx = client_resp_channels.remove(&log_index);
            tracing::debug!(
                log_id = debug(&log_id),
                membership = debug(&membership),
                "send_response"
            );

            if let Some(tx) = tx {
                let membership = membership;

                let res = Ok(ClientWriteResponse {
                    log_id,
                    data: resp,
                    membership,
                });

                tx.send(res);
            }
        }

        let resp = ApplyResult {
            since,
            end,
            last_applied,
        };

        Ok(resp)
    }

    /// Build a snapshot by requesting a builder from the state machine.
    ///
    /// This method calls
    /// [`try_create_snapshot_builder(false)`](`RaftStateMachine::try_create_snapshot_builder`)
    /// to allow the state machine to defer snapshot creation based on operational conditions.
    /// If deferred (`None` returned), a `BuildSnapshotDone(None)` response is sent to RaftCore.
    ///
    /// Building snapshot is a read-only operation that runs in a spawned task. This parallelization
    /// depends on the [`RaftSnapshotBuilder`] implementation: The builder must:
    /// - hold a consistent view of the state machine that won't be affected by further writes such
    ///   as applying a log entry,
    /// - or it must be able to acquire a lock that prevents any write operations.
    #[tracing::instrument(level = "info", skip_all)]
    async fn build_snapshot(&mut self, resp_tx: MpscUnboundedSenderOf<C, Notification<C>>) {
        // TODO: need to be abortable?
        // use futures::future::abortable;
        // let (fu, abort_handle) = abortable(async move { builder.build_snapshot().await });

        let builder = self.state_machine.try_create_snapshot_builder(false).await;

        let Some(mut builder) = builder else {
            tracing::info!("{}: snapshot building is refused by state machine", func_name!());
            let res = CommandResult::new(Ok(Response::BuildSnapshotDone(None)));
            resp_tx.send(Notification::sm(res)).ok();
            return;
        };

        let _handle = C::spawn(async move {
            let res = builder.build_snapshot().await;
            let res = res.map(|snap| Response::BuildSnapshotDone(Some(snap.meta)));
            let cmd_res = CommandResult::new(res);
            let _ = resp_tx.send(Notification::sm(cmd_res));
        });
        tracing::info!("{} returning; spawned building snapshot task", func_name!());
    }

    #[tracing::instrument(level = "info", skip_all)]
    async fn get_snapshot(&mut self, tx: OneshotSenderOf<C, Option<Snapshot<C>>>) -> Result<(), StorageError<C>> {
        tracing::info!("{}", func_name!());

        let snapshot = self.state_machine.get_current_snapshot().await?;

        tracing::info!(
            "sending back snapshot: meta: {}",
            snapshot.as_ref().map(|s| &s.meta).display()
        );
        let _ = tx.send(snapshot);
        Ok(())
    }
}
