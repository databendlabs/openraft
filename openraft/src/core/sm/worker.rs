use std::collections::BTreeMap;

use futures::TryStreamExt;
use tracing_futures::Instrument;

use crate::RaftLogReader;
use crate::RaftSnapshotBuilder;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::async_runtime::MpscUnboundedReceiver;
use crate::async_runtime::OneshotSender;
use crate::core::ApplyResult;
use crate::core::notification::Notification;
use crate::core::sm::Command;
use crate::core::sm::CommandResult;
use crate::core::sm::Response;
use crate::core::sm::handle::Handle;
use crate::display_ext::DisplayOptionExt;
use crate::entry::RaftEntry;
use crate::error::StorageIOResult;
use crate::raft::responder::core_responder::CoreResponder;
#[cfg(doc)]
use crate::storage::RaftLogStorage;
use crate::storage::RaftStateMachine;
use crate::storage::Snapshot;
use crate::storage::v2::entry_responder::EntryResponderBuilder;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::JoinHandleOf;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::MpscSenderOf;
use crate::type_config::alias::MpscUnboundedReceiverOf;
use crate::type_config::alias::OneshotSenderOf;
use crate::type_config::async_runtime::mpsc::MpscSender;

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
    resp_tx: MpscSenderOf<C, Notification<C>>,
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
        resp_tx: MpscSenderOf<C, Notification<C>>,
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

                let _ = self
                    .resp_tx
                    .send(Notification::StateMachine {
                        command_result: CommandResult { result: Err(err) },
                    })
                    .await;
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
                Command::InstallFullSnapshot {
                    log_io_id: io_id,
                    snapshot,
                } => {
                    tracing::info!("{}: install complete snapshot", func_name!());

                    let meta = snapshot.meta.clone();
                    self.state_machine
                        .install_snapshot(&meta, snapshot.snapshot)
                        .await
                        .sto_write_snapshot(Some(meta.signature()))?;

                    tracing::info!("Done install complete snapshot, meta: {}", meta);

                    let res = CommandResult::new(Ok(Response::InstallSnapshot((io_id, Some(meta)))));
                    self.resp_tx.send(Notification::sm(res)).await.ok();
                }
                Command::BeginReceivingSnapshot { tx } => {
                    tracing::info!("{}: BeginReceivingSnapshot", func_name!());

                    let snapshot_data = self.state_machine.begin_receiving_snapshot().await.sto_write_snapshot(None)?;

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
                    self.resp_tx.send(Notification::sm(res)).await.ok();
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
        client_resp_channels: &mut BTreeMap<u64, CoreResponder<C>>,
    ) -> Result<ApplyResult<C>, StorageError<C>> {
        let since = first.index();
        let end = last.index() + 1;

        #[cfg(debug_assertions)]
        let (got_last_index, last_apply) = {
            let l = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
            (l.clone(), l)
        };

        let strm = self.log_reader.entries_stream(since..end).await;

        // Prepare entries with responders upfront.
        let strm = strm.map_ok(move |entry| {
            let log_index = entry.index();
            let responder = client_resp_channels.remove(&log_index);
            let item = EntryResponderBuilder { entry, responder };

            #[cfg(debug_assertions)]
            last_apply.store(log_index, std::sync::atomic::Ordering::Relaxed);

            tracing::debug!("Applying entry to state machine: {}", item);

            let (ent, responder) = item.into_parts();

            (ent, responder)
        });

        self.state_machine.apply(Box::pin(strm)).await.sto_apply(last.clone())?;

        #[cfg(debug_assertions)]
        {
            assert_eq!(end - 1, got_last_index.load(std::sync::atomic::Ordering::Relaxed));
        }

        let resp = ApplyResult {
            since,
            end,
            last_applied: last,
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
    async fn build_snapshot(&mut self, resp_tx: MpscSenderOf<C, Notification<C>>) {
        // TODO: need to be abortable?
        // use futures::future::abortable;
        // let (fu, abort_handle) = abortable(async move { builder.build_snapshot().await });

        let builder = self.state_machine.try_create_snapshot_builder(false).await;

        let Some(mut builder) = builder else {
            tracing::info!("{}: snapshot building is refused by state machine", func_name!());
            let res = CommandResult::new(Ok(Response::BuildSnapshotDone(None)));
            resp_tx.send(Notification::sm(res)).await.ok();
            return;
        };

        let _handle = C::spawn(async move {
            let res = builder.build_snapshot().await.sto_write_snapshot(None);
            let res = res.map(|snap| Response::BuildSnapshotDone(Some(snap.meta)));
            let cmd_res = CommandResult::new(res);
            resp_tx.send(Notification::sm(cmd_res)).await.ok();
        });
        tracing::info!("{} returning; spawned building snapshot task", func_name!());
    }

    #[tracing::instrument(level = "info", skip_all)]
    async fn get_snapshot(&mut self, tx: OneshotSenderOf<C, Option<Snapshot<C>>>) -> Result<(), StorageError<C>> {
        tracing::info!("{}", func_name!());

        let snapshot = self.state_machine.get_current_snapshot().await.sto_read_snapshot(None)?;

        tracing::info!(
            "sending back snapshot: meta: {}",
            snapshot.as_ref().map(|s| &s.meta).display()
        );
        let _ = tx.send(snapshot);
        Ok(())
    }
}
