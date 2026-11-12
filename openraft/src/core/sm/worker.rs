use display_more::DisplayOptionExt;
use futures_util::StreamExt;
use futures_util::TryStreamExt;
use tracing::Instrument;

use crate::RaftLogReader;
use crate::RaftSnapshotBuilder;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::async_runtime::MpscReceiver;
use crate::async_runtime::OneshotSender;
use crate::async_runtime::mpsc::PeekableReceiver;
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
#[cfg(doc)]
use crate::storage::RaftLogStorage;
use crate::storage::RaftStateMachine;
use crate::storage::v2::entry_responder::EntryResponderBuilder;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::JoinHandleOf;
use crate::type_config::alias::MpscSenderOf;
use crate::type_config::alias::OneshotSenderOf;
use crate::type_config::alias::SnapshotOf;
use crate::type_config::async_runtime::mpsc::MpscSender;

pub(crate) struct Worker<C, SM, LR>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
    LR: RaftLogReader<C>,
{
    /// The ID of this Raft node, used to tag worker logs for correlation with core logs.
    id: C::NodeId,

    /// The application state machine implementation.
    state_machine: SM,

    /// Read logs from the [`RaftLogStorage`] implementation to apply them to the state machine.
    log_reader: LR,

    /// Read command from RaftCore to execute.
    ///
    /// Peekable so that collecting `Apply` commands into a batch can leave any other command
    /// queued.
    cmd_rx: PeekableReceiver<CommandReceiver<C, SM>, Command<C, SM>>,

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
        id: C::NodeId,
        state_machine: SM,
        log_reader: LR,
        resp_tx: MpscSenderOf<C, Notification<C>>,
        cmd_channel: (MpscSenderOf<C, Command<C, SM>>, CommandReceiver<C, SM>),
        span: tracing::Span,
    ) -> Handle<C, SM> {
        let (cmd_tx, cmd_rx) = cmd_channel;

        let worker = Worker {
            id,
            state_machine,
            log_reader,
            cmd_rx: cmd_rx.peekable(),
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

                self.resp_tx
                    .send(Notification::StateMachine {
                        command_result: CommandResult { result: Err(err) },
                    })
                    .await
                    .ok();
            }
        };
        C::spawn(fu.instrument(span))
    }

    /// Run commands in the order RaftCore sent them, reading each apply batch while the previous
    /// one is applying.
    ///
    /// At most two things are in flight: the batch being applied and the batch being read. The
    /// range of a batch is fixed when its read starts. Each round applies the batch read in the
    /// previous round and reads the `Apply` commands queued at that moment as the next batch, so
    /// the read overlaps the state-machine I/O. Commands that arrive during the round stay queued
    /// and become the batch after the next. Every batch is read into memory before it is applied,
    /// so `apply()` never borrows `log_reader` and one reader serves every read.
    #[tracing::instrument(level = "debug", skip_all)]
    async fn worker_loop(&mut self) -> Result<(), StorageError<C>> {
        let mut read_ahead: Option<ApplyBatch<C>> = None;

        loop {
            let batch = match read_ahead.take() {
                Some(batch) => batch,
                None => {
                    // Nothing was read ahead, so wait for a command. An `Apply` is read here,
                    // without an apply to overlap.
                    // The peeked reference must end before the awaits below: `Command` is not
                    // `Sync`, so holding it across an await would make the worker future `!Send`.
                    let apply_since = {
                        let Some(cmd) = self.cmd_rx.peek().await else {
                            tracing::info!("{}: rx closed, state machine worker quit", func_name!());
                            return Ok(());
                        };
                        tracing::debug!("RAFT_event id={:<2}  input: {:?}", self.id, cmd);
                        match cmd {
                            Command::Apply { first, .. } => Some(first.index()),
                            _ => None,
                        }
                    };
                    let Some(since) = apply_since else {
                        let cmd = self.cmd_rx.try_recv().expect("the peeked command is queued");
                        self.run_command(cmd).await?;
                        continue;
                    };
                    let mut batch = self.take_queued_applies(since).expect("the peeked command is an Apply");
                    Self::read(&mut self.log_reader, &mut batch).await;
                    batch
                }
            };

            let next = self.take_queued_applies(batch.end());
            let applied = match next {
                Some(mut next) => {
                    tracing::debug!("read ahead Apply: [{},{}]", next.first, next.last);
                    let apply = Self::apply(&mut self.state_machine, batch);
                    let read = Self::read(&mut self.log_reader, &mut next);
                    let (applied, ()) = futures_util::future::join(apply, read).await;
                    read_ahead = Some(next);
                    applied
                }
                None => Self::apply(&mut self.state_machine, batch).await,
            };

            let res = CommandResult::new(Ok(Response::Apply(applied?)));
            self.resp_tx.send(Notification::sm(res)).await.ok();
        }
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

    /// Take the `Apply` commands queued right now, starting at `since`, as one batch.
    ///
    /// RaftCore sends one `Apply` each time the committed index advances, so while the state
    /// machine is busy, many small `Apply` commands queue up. Applying them as one batch pays the
    /// per-call cost of `entries_stream()` and `apply()` once instead of once per command.
    ///
    /// Only commands that are already queued are taken. The first one that is not an `Apply`
    /// continuing the range stays in `cmd_rx` and runs next, so commands still run in the order
    /// RaftCore sent them. Returns `None` when no such `Apply` is queued.
    fn take_queued_applies(&mut self, since: u64) -> Option<ApplyBatch<C>> {
        let mut batch: Option<ApplyBatch<C>> = None;

        while let Ok(Command::Apply { first, .. }) = self.cmd_rx.try_peek()
            && first.index() == batch.as_ref().map_or(since, ApplyBatch::end)
        {
            let Ok(Command::Apply {
                first,
                last,
                client_resp_channels,
            }) = self.cmd_rx.try_recv()
            else {
                unreachable!("the peeked command is an Apply");
            };

            match &mut batch {
                None => batch = Some(ApplyBatch::new(first, last, client_resp_channels)),
                Some(batch) => {
                    tracing::debug!("merge queued Apply: [{},{}]", first, last);
                    batch.last = last;
                    batch.client_resp_channels.extend(client_resp_channels);
                }
            }
        }
        batch
    }

    /// Read the entries of `batch` into `batch.entries`.
    ///
    /// The buffer holds the entries of one batch, which is at most the committed but unapplied
    /// range that a single `Apply` command can already cover.
    async fn read(reader: &mut LR, batch: &mut ApplyBatch<C>) {
        let stream = reader.entries_stream(batch.since()..batch.end()).await;
        batch.entries = stream.collect().await;
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
}

#[cfg(test)]
mod worker_test;
