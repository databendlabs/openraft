use std::future::Future;
use std::future::pending;
use std::pin::Pin;

use display_more::DisplayOptionExt;
use futures_util::FutureExt;
use futures_util::StreamExt;
use futures_util::TryStreamExt;
use futures_util::future::MaybeDone;
use futures_util::future::maybe_done;
use tracing::Instrument;

use crate::OptionalSend;
use crate::RaftLogReader;
use crate::RaftSnapshotBuilder;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::async_runtime::MpscReceiver;
use crate::async_runtime::OneshotSender;
use crate::async_runtime::Select3;
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
use crate::type_config::alias::MpscSenderOf;
use crate::type_config::alias::OneshotSenderOf;
use crate::type_config::alias::SnapshotOf;
use crate::type_config::async_runtime::mpsc::MpscSender;

mod job;
mod pipeline;

use self::job::Job;
use self::pipeline::Pipeline;

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

    /// Coordinate retained input, owned reads, and owned state-machine jobs.
    #[tracing::instrument(level = "debug", skip_all, fields(id = %self.id))]
    async fn worker_loop<LR>(self, reader: LR, commands: CommandReceiver<C, SM>) -> Result<(), StorageError<C>>
    where LR: RaftLogReader<C> {
        let input = futures_util::stream::unfold(commands, Self::receive);
        let reading = None;
        let applying = None;
        futures_util::pin_mut!(input, reading, applying);
        let mut machine = Some(self);
        let mut reader = Some(reader);
        let mut pipeline = Pipeline::default();

        loop {
            if let Some((worker, result)) = Self::completed(applying.as_mut()) {
                result?;
                machine = Some(worker);
                pipeline.machine_finished();
            }
            pipeline.collect_ready(&mut input);
            if let Some((returned, batch)) = Self::completed(reading.as_mut()) {
                reader = Some(returned);
                pipeline.read_finished(batch);
            }

            // Completed reads were coalesced above. Loaded work never waits for new I/O.
            let machine_idle = machine.is_some();
            let reader_idle = reader.is_some();
            let next = pipeline.schedule(machine_idle, reader_idle);
            if let Some(job) = next {
                match job {
                    Job::Read(batch) => {
                        let owned = reader.take().expect("the reader is idle");
                        let future = Self::read_owned(owned, batch);
                        let future = maybe_done(future);
                        let scheduled = Some(future);
                        reading.set(scheduled);
                    }
                    job => {
                        let owned = machine.take().expect("the state machine is idle");
                        let future = owned.run_job(job);
                        let future = maybe_done(future);
                        let scheduled = Some(future);
                        applying.set(scheduled);
                    }
                }
                continue;
            }
            if pipeline.finished(machine_idle, reader_idle) {
                return Ok(());
            }
            let receive = pipeline.can_receive();
            let incoming = async {
                if !receive {
                    return pending().await;
                }
                input.next().await
            };
            let apply_slot = applying.as_mut();
            let apply_done = Self::wait_job(apply_slot);
            let read_slot = reading.as_mut();
            let read_done = Self::wait_job(read_slot);
            let selected = C::select3(apply_done, read_done, incoming).await;
            if let Select3::Third(command) = selected {
                pipeline.receive(command);
            }
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

    async fn read_owned<LR>(mut reader: LR, mut batch: ApplyBatch<C>) -> (LR, ApplyBatch<C>)
    where LR: RaftLogReader<C> {
        let since = batch.since();
        let end = batch.end();
        let range = since..end;
        let stream = reader.entries_stream(range).await;
        let entries = stream.collect().await;
        batch.entries = entries;
        (reader, batch)
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

    async fn run_job(mut self, job: Job<C, SM>) -> (Self, Result<(), StorageError<C>>) {
        let result = match job {
            Job::Apply(batch) => Self::apply_and_notify(&mut self.state_machine, &self.resp_tx, batch).await,
            Job::Command(command) => self.run_command(command).await,
            Job::Read(_) => unreachable!("reads own the reader instead of the state machine"),
        };
        (self, result)
    }

    async fn receive(mut receiver: CommandReceiver<C, SM>) -> Option<(Command<C, SM>, CommandReceiver<C, SM>)> {
        let command = receiver.recv().await?;
        Some((command, receiver))
    }

    fn completed<F>(mut slot: Pin<&mut Option<MaybeDone<F>>>) -> Option<F::Output>
    where F: Future {
        let mut future = slot.as_mut().as_pin_mut()?;
        future.as_mut().now_or_never()?;
        let output = future.take_output();
        slot.set(None);
        output
    }

    async fn wait_job<F>(slot: Pin<&mut Option<MaybeDone<F>>>)
    where F: Future + OptionalSend {
        match slot.as_pin_mut() {
            Some(future) => future.await,
            None => pending().await,
        }
    }
}

/// Only whole commands that fit this speculative entry budget are read during another apply.
const MAX_PREFETCH_ENTRIES: u64 = 1024;

#[cfg(test)]
mod worker_test;
