use display_more::DisplayOptionExt;
use futures_util::TryStreamExt;
use futures_util::future::try_join3;
use tracing::Instrument;

use crate::RaftLogReader;
use crate::RaftSnapshotBuilder;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::async_runtime::MpscReceiver;
use crate::async_runtime::OneshotSender;
use crate::async_runtime::watch::WatchSender;
use crate::core::ApplyResult;
use crate::core::notification::Notification;
use crate::core::sm::Command;
use crate::core::sm::CommandResult;
use crate::core::sm::Response;
use crate::core::sm::apply_batch::ApplyBatch;
use crate::core::sm::command_receiver::CommandReceiver;
use crate::core::sm::handle::Handle;
use crate::entry::RaftEntry;
use crate::errors::StorageIOResult;
use crate::storage::RaftStateMachine;
use crate::storage::v2::entry_responder::EntryResponderBuilder;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::JoinHandleOf;
use crate::type_config::alias::MpscReceiverOf;
use crate::type_config::alias::MpscSenderOf;
use crate::type_config::alias::OneshotSenderOf;
use crate::type_config::alias::SnapshotOf;
use crate::type_config::alias::WatchSenderOf;
use crate::type_config::async_runtime::mpsc::MpscSender;

mod intake;
mod progress;
mod reader;
mod work;

use self::intake::Intake;
use self::progress::Progress;
use self::reader::Reader;
use self::work::Work;

pub(crate) struct Worker<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    /// The ID of this Raft node, used to tag worker logs for correlation with core logs.
    id: C::NodeId,

    /// The application state machine implementation.
    state_machine: SM,

    /// Send back the result of the command to RaftCore.
    resp_tx: MpscSenderOf<C, Notification<C>>,
}

impl<C, SM> Worker<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    /// Spawn a new state machine worker, return a controlling handle.
    pub(crate) fn spawn<LR>(
        id: C::NodeId,
        state_machine: SM,
        log_reader: LR,
        resp_tx: MpscSenderOf<C, Notification<C>>,
        cmd_channel: (MpscSenderOf<C, Command<C, SM>>, CommandReceiver<C, SM>),
        span: tracing::Span,
    ) -> Handle<C, SM>
    where
        LR: RaftLogReader<C>,
    {
        let (cmd_tx, cmd_rx) = cmd_channel;

        let worker = Worker {
            id,
            state_machine,
            resp_tx,
        };

        let join_handle = worker.do_spawn(log_reader, cmd_rx, span);

        Handle { cmd_tx, join_handle }
    }

    fn do_spawn<LR>(self, reader: LR, commands: CommandReceiver<C, SM>, span: tracing::Span) -> JoinHandleOf<C, ()>
    where LR: RaftLogReader<C> {
        let errors = self.resp_tx.clone();
        let future = async move {
            if let Err(error) = self.worker_loop(reader, commands).await {
                tracing::error!(%error, "state machine worker failed");
                let result = CommandResult { result: Err(error) };
                let notification = Notification::StateMachine { command_result: result };
                errors.send(notification).await.ok();
            }
        };
        C::spawn(future.instrument(span))
    }

    /// Drive three owned, channel-connected stages in the existing worker task.
    #[tracing::instrument(level = "debug", skip_all, fields(id = %self.id))]
    async fn worker_loop<LR>(self, reader: LR, commands: CommandReceiver<C, SM>) -> Result<(), StorageError<C>>
    where LR: RaftLogReader<C> {
        let (read_tx, read_rx) = C::mpsc(STAGE_CHANNEL_SIZE);
        let (loaded_tx, loaded_rx) = C::mpsc(STAGE_CHANNEL_SIZE);
        let (progress_tx, progress_rx) = C::watch_channel(Progress::default());

        let intake_stage = Intake::new(commands, read_tx, progress_rx);
        let read_stage = Reader::new(reader, read_rx, loaded_tx);

        let intake_future = intake_stage.run();
        let read_future = read_stage.run();
        let apply_future = self.apply_loop(loaded_rx, progress_tx);

        try_join3(intake_future, read_future, apply_future).await?;
        Ok(())
    }

    /// Run a command other than `Apply`.
    async fn run_command(&mut self, cmd: Command<C, SM>) -> Result<(), StorageError<C>> {
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
            Command::Apply { .. } => {
                unreachable!("Apply is handled by the worker loop");
            }
            Command::ExternalFunc { func } => {
                tracing::debug!("{}: run user defined ExternalFunc", func_name!());
                func(&mut self.state_machine).await;
            }
        }
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn apply(state_machine: &mut SM, batch: ApplyBatch<C>) -> Result<ApplyResult<C>, StorageError<C>> {
        let since = batch.since();
        let end = batch.end();
        let last = batch.last;

        #[cfg(debug_assertions)]
        let (got_last_index, last_apply) = {
            let l = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
            (l.clone(), l)
        };

        let strm = futures_util::stream::iter(batch.entries);

        // Convert Vec to an iterator for efficient matching
        let mut responder_iter = batch.client_resp_channels.into_iter().peekable();

        // Prepare entries with responders upfront.
        let strm = strm.map_ok(move |entry| {
            let log_index = entry.index();

            // Check if the next responder matches this log index
            let responder = if responder_iter.peek().map(|(idx, _)| *idx) == Some(log_index) {
                responder_iter.next().map(|(_, r)| r)
            } else {
                None
            };

            let item = EntryResponderBuilder { entry, responder };

            #[cfg(debug_assertions)]
            last_apply.store(log_index, std::sync::atomic::Ordering::Relaxed);

            tracing::debug!("Applying entry to state machine: {}", item);

            let (ent, responder) = item.into_parts();

            (ent, responder)
        });

        state_machine.apply(Box::pin(strm)).await.sto_apply(last.clone())?;

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
        tracing::info!("{}: returning; spawned building snapshot task", func_name!());
    }

    #[tracing::instrument(level = "info", skip_all)]
    async fn get_snapshot(
        &mut self,
        tx: OneshotSenderOf<C, Option<SnapshotOf<C, SM::SnapshotData>>>,
    ) -> Result<(), StorageError<C>> {
        tracing::info!("{}", func_name!());

        let snapshot = self.state_machine.get_current_snapshot().await.sto_read_snapshot(None)?;

        tracing::info!(
            "sending back snapshot: meta: {}",
            snapshot.as_ref().map(|s| &s.meta).display()
        );
        tx.send(snapshot).ok();
        Ok(())
    }

    /// Publish a completed apply even while the next batch is still being read.
    async fn apply_and_notify(
        state_machine: &mut SM,
        resp_tx: &MpscSenderOf<C, Notification<C>>,
        batch: ApplyBatch<C>,
    ) -> Result<(), StorageError<C>> {
        let applied = Self::apply(state_machine, batch).await?;
        let res = CommandResult::new(Ok(Response::Apply(applied)));
        resp_tx.send(Notification::sm(res)).await.ok();
        Ok(())
    }
    async fn apply_loop(
        mut self,
        incoming: MpscReceiverOf<C, Work<C, SM>>,
        progress_tx: WatchSenderOf<C, Progress>,
    ) -> Result<(), StorageError<C>> {
        let mut incoming = incoming.peekable();
        let mut progress = Progress::default();
        while let Some(mut work) = incoming.recv().await {
            work.coalesce(&mut incoming);
            match work {
                Work::Apply(batch) => {
                    let entries = batch.entry_count();
                    progress.started += entries;
                    progress_tx.send(progress).ok();
                    Self::apply_and_notify(&mut self.state_machine, &self.resp_tx, batch).await?;
                    progress.finished += entries;
                    progress_tx.send(progress).ok();
                }
                Work::Barrier { command, finished } => {
                    self.run_command(command).await?;
                    finished.send(()).ok();
                }
            }
        }
        Ok(())
    }
}

/// Only whole commands that fit this speculative entry budget are read during another apply.
const MAX_PREFETCH_ENTRIES: u64 = 1024;

/// Entry credits bound one-entry messages across both queues as well as active reads.
const STAGE_CHANNEL_SIZE: usize = MAX_PREFETCH_ENTRIES as usize;

#[cfg(test)]
mod worker_test;
