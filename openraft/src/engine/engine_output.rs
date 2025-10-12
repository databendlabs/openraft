use std::collections::VecDeque;

use crate::RaftTypeConfig;
use crate::engine::Command;
use crate::engine::Condition;
use crate::engine::pending_responds::PendingResponds;
use crate::engine::respond_command::PendingRespond;

/// The entry of output from Engine to the runtime.
#[derive(Debug, Default)]
pub(crate) struct EngineOutput<C>
where C: RaftTypeConfig
{
    /// Command queue that needs to be executed by `RaftRuntime`.
    pub(crate) commands: VecDeque<Command<C>>,

    /// Pending responds waiting for IO conditions to be met before sending.
    pub(crate) pending_responds: PendingResponds<C>,
}

impl<C> EngineOutput<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(command_buffer_size: usize) -> Self {
        let pending_capacity = 1024;
        Self {
            commands: VecDeque::with_capacity(command_buffer_size),
            pending_responds: PendingResponds::new(pending_capacity),
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

    /// Put the command to the head of the queue or to a separate pending queue.
    ///
    /// This will be used when the command is not ready to be executed.
    ///
    /// Returns Ok if the cmd is put to a pending queue, means it is not put back, and other
    /// commands in the main queue can still be processed.
    pub(crate) fn postpone_command(&mut self, cmd: Command<C>) -> Result<(), &'static str> {
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
