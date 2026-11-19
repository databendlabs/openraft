use super::MAX_PREFETCH_ENTRIES;
use super::progress::Progress;
use super::work::Work;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::async_runtime::MpscReceiver;
use crate::async_runtime::MpscSender;
use crate::async_runtime::mpsc::PeekableReceiver;
use crate::async_runtime::watch::WatchReceiver;
use crate::core::sm::Command;
use crate::core::sm::apply_batch::ApplyBatch;
use crate::core::sm::command_receiver::CommandReceiver;
use crate::storage::RaftStateMachine;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::MpscSenderOf;
use crate::type_config::alias::WatchReceiverOf;

/// Own command intake, reserving an entry budget before work reaches either later stage.
pub(super) struct Intake<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    commands: PeekableReceiver<CommandReceiver<C, SM>, Command<C, SM>>,
    output: MpscSenderOf<C, Work<C, SM>>,
    progress: WatchReceiverOf<C, Progress>,
    admitted: u64,
}

impl<C, SM> Intake<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    pub(super) fn new(
        commands: CommandReceiver<C, SM>,
        output: MpscSenderOf<C, Work<C, SM>>,
        progress: WatchReceiverOf<C, Progress>,
    ) -> Self {
        Self {
            commands: commands.peekable(),
            output,
            progress,
            admitted: 0,
        }
    }

    pub(super) async fn run(mut self) -> Result<(), StorageError<C>> {
        while let Some(command) = self.commands.recv().await {
            if let Command::Apply {
                first,
                last,
                client_resp_channels,
            } = command
            {
                let mut batch = ApplyBatch::new(first, last, client_resp_channels);
                let entries = batch.entry_count();
                let reserved = self.reserve(entries).await;
                if !reserved {
                    return Ok(());
                }
                self.coalesce(&mut batch);
                let work = Work::Apply(batch);
                let sent = self.output.send(work).await;
                if sent.is_err() {
                    return Ok(());
                }
            } else {
                let acknowledged = self.barrier(command).await;
                if !acknowledged {
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    fn available(&mut self, entries: u64) -> bool {
        let progress = *self.progress.borrow_and_update();
        if entries > MAX_PREFETCH_ENTRIES {
            return self.admitted == progress.started && progress.started == progress.finished;
        }
        let reserved = self.admitted - progress.started;
        let remaining = MAX_PREFETCH_ENTRIES.saturating_sub(reserved);
        entries <= remaining
    }

    async fn reserve(&mut self, entries: u64) -> bool {
        while !self.available(entries) {
            let changed = self.progress.changed().await;
            if changed.is_err() {
                return false;
            }
        }
        self.admitted += entries;
        true
    }

    fn coalesce(&mut self, batch: &mut ApplyBatch<C>) {
        while let Ok(Command::Apply { first, last, .. }) = self.commands.try_peek() {
            let since = first.index();
            let end = last.index() + 1;
            let entries = end - since;
            if since != batch.end() || !self.available(entries) {
                break;
            }
            let Ok(Command::Apply {
                first,
                last,
                client_resp_channels,
            }) = self.commands.try_recv()
            else {
                unreachable!("the peeked command remains in the receiver");
            };
            self.admitted += entries;
            let next = ApplyBatch::new(first, last, client_resp_channels);
            batch.extend(next);
        }
    }

    async fn barrier(&mut self, command: Command<C, SM>) -> bool {
        let (finished, acknowledged) = C::oneshot();
        let work = Work::Barrier { command, finished };
        let sent = self.output.send(work).await;
        if sent.is_err() {
            return false;
        }
        let result = acknowledged.await;
        result.is_ok()
    }
}
