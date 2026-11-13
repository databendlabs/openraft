use std::collections::VecDeque;

use crate::RaftTypeConfig;
use crate::config::Config;
use crate::engine::Command;
use crate::engine::Condition;
use crate::engine::command_scheduler::CommandScheduler;
use crate::engine::pending_responds::PendingResponds;
use crate::engine::respond_command::PendingRespond;
use crate::storage::RaftStateMachine;

pub(crate) const INITIAL_QUEUE_CAPACITY: usize = 64;

/// The entry of output from Engine to the runtime.
#[derive(Debug)]
pub(crate) struct EngineOutput<C, SM = ()>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    /// Command queue that needs to be executed by `RaftRuntime`.
    pub(crate) commands: VecDeque<Command<C, SM>>,

    /// Pending responds waiting for IO conditions to be met before sending.
    pub(crate) pending_responds: PendingResponds<C>,
}

impl<C, SM> Default for EngineOutput<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    fn default() -> Self {
        Self {
            commands: VecDeque::new(),
            pending_responds: PendingResponds::default(),
        }
    }
}

impl<C, SM> EngineOutput<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    pub(crate) fn new(command_buffer_size: usize) -> Self {
        let pending_capacity = INITIAL_QUEUE_CAPACITY;
        Self {
            commands: VecDeque::with_capacity(command_buffer_size),
            pending_responds: PendingResponds::new(pending_capacity),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.commands.len()
    }

    /// Push a command to the queue.
    pub(crate) fn push_command(&mut self, cmd: Command<C, SM>) {
        tracing::debug!("push command: {:?}", cmd);
        self.commands.push_back(cmd)
    }

    pub(crate) fn prepend_command(&mut self, cmd: Command<C, SM>) {
        tracing::debug!("push front command: {:?}", cmd);
        self.commands.push_front(cmd)
    }

    /// Put the command to the head of the queue or to a separate pending queue.
    ///
    /// This will be used when the command is not ready to be executed.
    ///
    /// Returns Ok if the cmd is put to a pending queue, means it is not put back, and other
    /// commands in the main queue can still be processed.
    pub(crate) fn postpone_command(&mut self, cmd: Command<C, SM>) -> Result<(), &'static str> {
        tracing::debug!("postpone command: {:?}", cmd);

        // For Respond command, put them to separate queue in order not to block other commands.
        // For other commands, put them back to the front of the command queue.
        match cmd {
            Command::Respond { when, resp } => {
                let pending_responds = &mut self.pending_responds;
                match when {
                    None => {
                        unreachable!("Respond command to postpone must have a condition");
                    }
                    Some(Condition::IOFlushed { io_id }) => {
                        pending_responds.on_log_io.push_back(PendingRespond::new(io_id, resp));
                    }
                    Some(Condition::LogFlushed { log_id }) => {
                        pending_responds.on_log_flush.push_back(PendingRespond::new(log_id, resp));
                    }
                    Some(Condition::Applied { log_id }) => {
                        pending_responds.on_apply.push_back(PendingRespond::new(log_id, resp));
                    }
                    Some(Condition::Snapshot { log_id }) => {
                        pending_responds.on_snapshot.push_back(PendingRespond::new(log_id, resp));
                    }
                }
                Ok(())
            }

            _ => {
                self.commands.push_front(cmd);
                Err("Put back to the front of command queue")
            }
        }
    }

    /// Pop the first command to run from the queue.
    pub(crate) fn pop_command(&mut self) -> Option<Command<C, SM>> {
        let cmd = self.commands.pop_front();
        // A drained VecDeque keeps its head where the last pop left it, so each
        // command would write the next slot until the whole preallocated buffer is
        // resident. `clear()` on an empty queue moves the head back to slot 0.
        if self.commands.is_empty() {
            self.commands.clear();
        }
        cmd
    }

    /// Iterate all queued commands.
    pub(crate) fn iter_commands(&self) -> impl Iterator<Item = &Command<C, SM>> {
        self.commands.iter()
    }

    /// Take all queued commands and clear the queue.
    #[cfg(test)]
    pub(crate) fn take_commands(&mut self) -> Vec<Command<C, SM>> {
        self.commands.drain(..).collect()
    }

    /// Clear all queued commands.
    #[cfg(any(test, feature = "bench"))]
    pub(crate) fn clear_commands(&mut self) {
        self.commands.clear()
    }

    /// Reorganize commands for better I/O performance.
    ///
    /// Currently performs:
    /// - Merging consecutive `AppendEntries` commands from the same leader into one
    ///
    /// The batch size is limited by `config.max_in_batch_entries` to avoid high latency
    /// from writing very large batches to storage. A value of 0 means unlimited batching.
    pub(crate) fn sched_commands(&mut self, config: &Config) {
        let mut scheduler = CommandScheduler::new(config, self);
        scheduler.merge_front_append_entries();
    }
}

#[cfg(test)]
mod tests {
    use super::EngineOutput;
    use crate::batch::Batch;
    use crate::engine::Command;
    use crate::engine::testing::UTConfig;
    use crate::engine::testing::UTLeaderId;
    use crate::impls::Vote;
    use crate::testing::blank_ent;
    use crate::vote::raft_vote::RaftVoteExt;

    fn append(index: u64) -> Command<UTConfig> {
        Command::AppendEntries {
            committed_vote: Vote::<UTLeaderId>::new(1, 0).to_committed(),
            entries: Batch::of([blank_ent::<UTConfig>(1, 0, index)]),
        }
    }

    #[test]
    fn test_pop_command_reuses_slot_zero_once_empty() {
        let mut output: EngineOutput<UTConfig> = EngineOutput::new(8);
        output.push_command(append(1));
        let first_slot = output.commands.as_slices().0.as_ptr();

        assert!(output.pop_command().is_some());
        output.push_command(append(2));
        assert_eq!(output.commands.as_slices().0.as_ptr(), first_slot);
    }

    #[test]
    fn test_commands_grow_and_retain_capacity() {
        let mut output: EngineOutput<UTConfig> = EngineOutput::new(1);
        output.push_command(append(1));
        output.push_command(append(2));
        let capacity = output.commands.capacity();
        let grew = capacity > 1;
        assert!(grew);

        let mut commands = Vec::new();
        while let Some(command) = output.pop_command() {
            commands.push(command);
        }
        let expected = vec![append(1), append(2)];
        assert_eq!(expected, commands);
        let retained_capacity = output.commands.capacity();
        assert_eq!(capacity, retained_capacity);
    }
}
