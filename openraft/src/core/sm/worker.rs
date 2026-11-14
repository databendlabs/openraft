use std::pin::pin;

use display_more::DisplayOptionExt;
use futures_util::StreamExt;
use futures_util::TryStreamExt;
use futures_util::future::Either;
use tracing::Instrument;

use crate::RaftLogReader;
use crate::RaftSnapshotBuilder;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::async_runtime::OneshotSender;
use crate::async_runtime::Select2;
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

    /// Read the next apply batch while the current one is applying.
    ///
    /// It is a second reader because the entry stream of the current batch may borrow
    /// `log_reader` until `apply()` returns.
    prefetch_reader: LR,

    /// Read command from RaftCore to execute.
    cmd_rx: CommandReceiver<C, SM>,

    /// Send back the result of the command to RaftCore.
    resp_tx: MpscSenderOf<C, Notification<C>>,

    /// A command taken from `cmd_rx` while merging `Apply` commands; it runs next.
    next_cmd: Option<Command<C, SM>>,
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
        prefetch_reader: LR,
        resp_tx: MpscSenderOf<C, Notification<C>>,
        cmd_channel: (MpscSenderOf<C, Command<C, SM>>, CommandReceiver<C, SM>),
        span: tracing::Span,
    ) -> Handle<C, SM> {
        let (cmd_tx, cmd_rx) = cmd_channel;

        let worker = Worker {
            id,
            state_machine,
            log_reader,
            prefetch_reader,
            cmd_rx,
            resp_tx,
            next_cmd: None,
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

    #[tracing::instrument(level = "debug", skip_all)]
    async fn worker_loop(&mut self) -> Result<(), StorageError<C>> {
        loop {
            let cmd = match self.next_cmd.take() {
                Some(cmd) => Some(cmd),
                None => self.cmd_rx.recv().await,
            };
            let cmd = match cmd {
                None => {
                    tracing::info!("{}: rx closed, state machine worker quit", func_name!());
                    return Ok(());
                }
                Some(x) => x,
            };

            tracing::debug!("RAFT_event id={:<2}  input: {:?}", self.id, cmd);

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
                Command::Apply {
                    first,
                    last,
                    client_resp_channels,
                } => {
                    let mut batch = ApplyBatch::new(first, last, client_resp_channels);
                    Self::merge_queued_applies(&mut self.cmd_rx, &mut self.next_cmd, &mut batch);
                    self.run_applies(batch).await?;
                }
                Command::ExternalFunc { func } => {
                    tracing::debug!("{}: run user defined ExternalFunc", func_name!());
                    func(&mut self.state_machine).await;
                }
            };
        }
    }
    /// Apply `batch`, then every batch prefetched while the previous one was applying.
    async fn run_applies(&mut self, batch: ApplyBatch<C>) -> Result<(), StorageError<C>> {
        let mut next = Some(batch);
        while let Some(batch) = next {
            let (applied, prefetched) = self.apply_and_prefetch(batch).await;
            let res = CommandResult::new(Ok(Response::Apply(applied?)));
            self.resp_tx.send(Notification::sm(res)).await.ok();
            next = prefetched;
        }
        Ok(())
    }

    /// Apply `batch`, and read the next batch while it is applying.
    ///
    /// While `apply()` runs, the worker keeps receiving commands. Each `Apply` that continues the
    /// range is merged with the `Apply` commands queued after it, its entries are read with
    /// `prefetch_reader`, and it is appended to the next batch. The next batch thus collects every
    /// `Apply` that arrives during `apply()`, as merging alone would, but its reads overlap the
    /// state-machine I/O. The first command of another kind is kept in `next_cmd` and runs after
    /// the next batch, so commands still run in the order RaftCore sent them.
    ///
    /// A read that is in progress when `apply()` finishes is completed, so the next batch starts
    /// at most one read later than without prefetching.
    ///
    /// Returns the result of applying `batch` and the prefetched batch, if any.
    async fn apply_and_prefetch(
        &mut self,
        batch: ApplyBatch<C>,
    ) -> (Result<ApplyResult<C>, StorageError<C>>, Option<ApplyBatch<C>>) {
        let mut next_since = batch.end();
        let apply = Self::apply(&mut self.state_machine, &mut self.log_reader, batch);
        let mut apply = pin!(apply);
        let mut prefetched: Option<ApplyBatch<C>> = None;

        while self.next_cmd.is_none() {
            // `recv()` is dropped when `apply()` finishes first, as RaftCore does with its own
            // receivers; a command not yet received stays in the channel.
            let received = C::select2(apply.as_mut(), self.cmd_rx.recv()).await;
            let cmd = match received {
                Select2::First(applied) => return (applied, prefetched),
                Select2::Second(cmd) => cmd,
            };

            let mut part = match cmd {
                Some(Command::Apply {
                    first,
                    last,
                    client_resp_channels,
                }) if first.index() == next_since => ApplyBatch::new(first, last, client_resp_channels),
                Some(cmd) => {
                    self.next_cmd = Some(cmd);
                    break;
                }
                // The channel is closed; the worker loop sees it after this batch.
                None => break,
            };

            Self::merge_queued_applies(&mut self.cmd_rx, &mut self.next_cmd, &mut part);
            tracing::debug!("prefetch Apply: [{},{}]", part.first, part.last);
            next_since = part.end();

            let applied = {
                let read = Self::prefetch(&mut self.prefetch_reader, &mut part);
                let mut read = pin!(read);
                match C::select2(apply.as_mut(), read.as_mut()).await {
                    Select2::First(applied) => {
                        read.await;
                        Some(applied)
                    }
                    Select2::Second(()) => None,
                }
            };

            match &mut prefetched {
                Some(prefetched) => prefetched.append(part),
                None => prefetched = Some(part),
            }

            if let Some(applied) = applied {
                return (applied, prefetched);
            }
        }

        (apply.await, prefetched)
    }

    /// Extend `batch` with the `Apply` commands queued right after it.
    ///
    /// RaftCore sends one `Apply` each time the committed index advances, so while the state
    /// machine is busy, many small `Apply` commands queue up. Applying them as one batch pays the
    /// per-call cost of `entries_stream()` and `apply()` once instead of once per command.
    ///
    /// Only commands that are already queued are taken. The first one that is not an `Apply`
    /// starting right after `batch` is kept in `next_cmd` and runs next, so commands still run in
    /// the order RaftCore sent them.
    fn merge_queued_applies(
        cmd_rx: &mut CommandReceiver<C, SM>,
        next_cmd: &mut Option<Command<C, SM>>,
        batch: &mut ApplyBatch<C>,
    ) {
        while let Some(cmd) = cmd_rx.try_recv() {
            match cmd {
                Command::Apply {
                    first,
                    last,
                    client_resp_channels,
                } if first.index() == batch.end() => {
                    tracing::debug!("merge queued Apply: [{},{}]", first, last);
                    batch.last = last;
                    batch.client_resp_channels.extend(client_resp_channels);
                }
                cmd => {
                    *next_cmd = Some(cmd);
                    break;
                }
            }
        }
    }

    /// Read all entries of `batch` into `batch.prefetched`.
    ///
    /// The buffer holds at most the entries of one batch, which is no more than the committed but
    /// unapplied range a single `Apply` command can already cover.
    async fn prefetch(reader: &mut LR, batch: &mut ApplyBatch<C>) {
        let stream = reader.entries_stream(batch.since()..batch.end()).await;
        let entries = stream.collect::<Vec<_>>().await;
        batch.prefetched = Some(entries);
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn apply(
        state_machine: &mut SM,
        log_reader: &mut LR,
        batch: ApplyBatch<C>,
    ) -> Result<ApplyResult<C>, StorageError<C>> {
        let since = batch.since();
        let end = batch.end();
        let last = batch.last;

        #[cfg(debug_assertions)]
        let (got_last_index, last_apply) = {
            let l = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
            (l.clone(), l)
        };

        let strm = match batch.prefetched {
            Some(entries) => Either::Left(futures_util::stream::iter(entries)),
            None => Either::Right(log_reader.entries_stream(since..end).await),
        };

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
