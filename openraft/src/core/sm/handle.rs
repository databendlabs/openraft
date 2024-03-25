//! State machine control handle

use tokio::sync::mpsc;

use crate::core::sm::Command;
use crate::type_config::alias::JoinHandleOf;
use crate::RaftTypeConfig;

/// State machine worker handle for sending command to it.
pub(crate) struct Handle<C>
where C: RaftTypeConfig
{
    pub(in crate::core::sm) cmd_tx: mpsc::UnboundedSender<Command<C>>,

    #[allow(dead_code)]
    pub(in crate::core::sm) join_handle: JoinHandleOf<C, ()>,
}

impl<C> Handle<C>
where C: RaftTypeConfig
{
    pub(crate) fn send(&mut self, cmd: Command<C>) -> Result<(), mpsc::error::SendError<Command<C>>> {
        tracing::debug!("sending command to state machine worker: {:?}", cmd);
        self.cmd_tx.send(cmd)
    }
}
