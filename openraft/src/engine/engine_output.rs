use std::collections::VecDeque;

use crate::RaftTypeConfig;
use crate::engine::Command;

/// The entry of output from Engine to the runtime.
#[derive(Debug, Default)]
pub(crate) struct EngineOutput<C>
where C: RaftTypeConfig
{
    /// Command queue that needs to be executed by `RaftRuntime`.
    pub(crate) commands: VecDeque<Command<C>>,
}

impl<C> EngineOutput<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(command_buffer_size: usize) -> Self {
        Self {
            commands: VecDeque::with_capacity(command_buffer_size),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.commands.len()
    }

    /// Push a command to the queue.
    pub(crate) fn push_command(&mut self, cmd: Command<C>) {
        tracing::debug!("push command: {:?}", cmd);
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
