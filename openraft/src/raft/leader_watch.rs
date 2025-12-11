//! Leader change watch handle.

use crate::RaftTypeConfig;
use crate::type_config::alias::JoinHandleOf;
use crate::type_config::alias::OneshotSenderOf;

/// Handle to control a leader watch task.
///
/// Use [`close()`](`Self::close`) to stop watching and wait for the task to complete.
pub struct LeaderWatchHandle<C>
where C: RaftTypeConfig
{
    pub(crate) cancel_tx: Option<OneshotSenderOf<C, ()>>,
    pub(crate) join_handle: Option<JoinHandleOf<C, ()>>,
}

impl<C> LeaderWatchHandle<C>
where C: RaftTypeConfig
{
    /// Stop watching and wait for the task to complete.
    pub async fn close(&mut self) {
        // Drop the sender to signal shutdown
        drop(self.cancel_tx.take());

        // Wait for task to finish
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.await;
        }
    }
}
