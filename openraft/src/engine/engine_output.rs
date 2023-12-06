use std::collections::VecDeque;

use crate::core::sm::CommandSeq;
use crate::engine::Command;
use crate::RaftTypeConfig;

/// The entry of output from Engine to the runtime.
#[derive(Debug, Default)]
pub(crate) struct EngineOutput<C>
where C: RaftTypeConfig
{
    /// A Engine level sequence number for identifying a command.
    pub(crate) seq: CommandSeq,

    /// Command queue that need to be executed by `RaftRuntime`.
    pub(crate) commands: VecDeque<Command<C>>,
}

impl<C> EngineOutput<C>
where C: RaftTypeConfig
{
    /// Generate the next command seq of an sm::Command.
    pub(crate) fn next_sm_seq(&mut self) -> CommandSeq {
        self.seq += 1;
        self.seq
    }

    /// Get the last used sm::Command seq
    pub(crate) fn last_sm_seq(&self) -> CommandSeq {
        self.seq
    }

    pub(crate) fn new(command_buffer_size: usize) -> Self {
        Self {
            seq: 0,
            commands: VecDeque::with_capacity(command_buffer_size),
        }
    }

    /// Push a command to the queue.
    pub(crate) fn push_command(&mut self, mut cmd: Command<C>) {
        match &mut cmd {
            Command::StateMachine { command } => {
                let seq = self.next_sm_seq();
                tracing::debug!("next_seq: {}", seq);
                command.set_seq(seq);
            }
            Command::BecomeLeader => {}
            Command::QuitLeader => {}
            Command::AppendEntry { .. } => {}
            Command::AppendInputEntries { .. } => {}
            Command::ReplicateCommitted { .. } => {}
            Command::Commit { .. } => {}
            Command::Replicate { .. } => {}
            Command::RebuildReplicationStreams { .. } => {}
            Command::SaveVote { .. } => {}
            Command::SendVote { .. } => {}
            Command::PurgeLog { .. } => {}
            Command::DeleteConflictLog { .. } => {}
            Command::Respond { .. } => {}
        }
        self.commands.push_back(cmd)
    }

    /// Put back the command to the head of the queue.
    ///
    /// This will be used when the command is not ready to be executed.
    pub(crate) fn postpone_command(&mut self, cmd: Command<C>) {
        tracing::debug!("postpone command: {:?}", cmd);
        self.commands.push_front(cmd)
    }

    /// Pop the first command to run from the queue.
    pub(crate) fn pop_command(&mut self) -> Option<Command<C>> {
        self.commands.pop_front()
    }

    /// Iterate all queued commands.
    pub(crate) fn iter_commands(&self) -> impl Iterator<Item = &Command<C>> {
        self.commands.iter()
    }

    /// Take all queued commands and clear the queue.
    #[cfg(test)]
    pub(crate) fn take_commands(&mut self) -> Vec<Command<C>> {
        self.commands.drain(..).collect()
    }

    /// Clear all queued commands.
    #[cfg(test)]
    pub(crate) fn clear_commands(&mut self) {
        self.commands.clear()
    }
}
