use std::collections::VecDeque;

use crate::engine::Command;
use crate::entry::RaftEntry;
use crate::MetricsChangeFlags;
use crate::Node;
use crate::NodeId;

/// The entry of output from Engine to the runtime.
#[derive(Debug, Default)]
pub(crate) struct EngineOutput<NID, N, Ent>
where
    NID: NodeId,
    N: Node,
    Ent: RaftEntry<NID, N>,
{
    /// Tracks what kind of metrics changed
    pub(crate) metrics_flags: MetricsChangeFlags,

    /// Command queue that need to be executed by `RaftRuntime`.
    pub(crate) commands: VecDeque<Command<NID, N, Ent>>,
}

impl<NID, N, Ent> EngineOutput<NID, N, Ent>
where
    NID: NodeId,
    N: Node,
    Ent: RaftEntry<NID, N>,
{
    pub(crate) fn new(command_buffer_size: usize) -> Self {
        Self {
            metrics_flags: MetricsChangeFlags::default(),
            commands: VecDeque::with_capacity(command_buffer_size),
        }
    }

    /// Push a command to the queue.
    pub(crate) fn push_command(&mut self, cmd: Command<NID, N, Ent>) {
        cmd.update_metrics_flags(&mut self.metrics_flags);
        self.commands.push_back(cmd)
    }

    /// Put back the command to the head of the queue.
    ///
    /// This will be used when the command is not ready to be executed.
    pub(crate) fn postpone_command(&mut self, cmd: Command<NID, N, Ent>) {
        tracing::debug!("postpone command: {:?}", cmd);
        self.commands.push_front(cmd)
    }

    /// Pop the first command to run from the queue.
    pub(crate) fn pop_command(&mut self) -> Option<Command<NID, N, Ent>> {
        self.commands.pop_front()
    }

    /// Iterate all queued commands.
    pub(crate) fn iter_commands(&self) -> impl Iterator<Item = &Command<NID, N, Ent>> {
        self.commands.iter()
    }

    /// Take all queued commands and clear the queue.
    #[cfg(test)]
    pub(crate) fn take_commands(&mut self) -> Vec<Command<NID, N, Ent>> {
        self.commands.drain(..).collect()
    }

    /// Clear all queued commands.
    #[cfg(test)]
    pub(crate) fn clear_commands(&mut self) {
        self.commands.clear()
    }
}
