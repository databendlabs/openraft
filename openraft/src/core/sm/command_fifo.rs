use std::collections::VecDeque;
use std::future::Future;

use crate::RaftTypeConfig;
use crate::StorageError;
use crate::base::BoxFuture;
use crate::core::sm;
use crate::core::sm::handle::Handle;
use crate::storage::RaftStateMachine;

/// State-machine commands issued by [`RaftCore`] that have not yet entered the worker channel.
///
/// [`RaftCore`] queues a command here instead of waiting for room in the worker channel, because
/// the worker may itself wait for notification capacity that only [`RaftCore`] frees. Commands
/// enter the worker channel oldest first. The send of the oldest command is kept until it
/// completes, so it keeps its command and its place among the senders waiting for room.
///
/// [`RaftCore`]: crate::core::RaftCore
pub(crate) struct CommandFifo<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    /// The most commands the FIFO holds, including the one being sent.
    capacity: usize,

    /// Commands waiting behind the one being sent, oldest first.
    queued: VecDeque<sm::Command<C, SM>>,

    /// The send of the oldest command, kept until the command enters the worker channel.
    sending: Option<BoxFuture<'static, Result<(), StorageError<C>>>>,
}

impl<C, SM> CommandFifo<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            capacity,
            queued: VecDeque::new(),
            sending: None,
        }
    }

    /// Returns whether the FIFO holds `capacity` commands, counting the one being sent.
    pub(crate) fn is_full(&self) -> bool {
        let sending = usize::from(self.sending.is_some());
        let held = self.queued.len() + sending;
        held >= self.capacity
    }

    /// Queues `cmd` behind every command queued before it.
    ///
    /// The caller checks [`Self::is_full`] before preparing the command.
    pub(crate) fn push(&mut self, cmd: sm::Command<C, SM>) {
        debug_assert!(!self.is_full(), "push to a full state machine command FIFO: {}", cmd);
        self.queued.push_back(cmd);
    }

    /// Returns a future that completes when the oldest command has entered the worker channel.
    ///
    /// It starts sending the oldest queued command unless a send is in progress, and stays
    /// pending while the FIFO is empty. The send is kept in `self`, so dropping the returned
    /// future, for example when another `select` branch wins, leaves the send in place.
    pub(crate) fn send_oldest(
        &mut self,
        handle: &Handle<C, SM>,
    ) -> impl Future<Output = Result<(), StorageError<C>>> + use<'_, C, SM> {
        if self.sending.is_none()
            && let Some(cmd) = self.queued.pop_front()
        {
            let send = handle.send(cmd);
            self.sending = Some(Box::pin(send));
        }

        async move {
            let Some(sending) = self.sending.as_mut() else {
                return std::future::pending().await;
            };
            let res = sending.await;
            self.sending = None;
            res
        }
    }
}

#[cfg(test)]
mod command_fifo_test;
