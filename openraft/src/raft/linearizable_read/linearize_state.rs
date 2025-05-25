use std::fmt;

use openraft_macros::since;

use crate::display_ext::DisplayOptionExt;
use crate::LogId;
use crate::RaftTypeConfig;

/// Represents the state after awaiting the applied log entries for a linearizable read.
///
/// This is returned by [`Linearizer::await_applied()`] after waiting for the state
/// machine to apply all necessary log entries for a linearizable read.
///
/// This struct contains the log IDs that were used to ensure linearizability:
/// - `read_log_id`: the log ID that was required to be applied before reading
/// - `applied`: the actual log ID that was applied to satisfy the linearizability requirement
///
/// If the state is ready, it is guaranteed `applied >= read_log_id`.
/// If the state is not ready(timeout waiting for the applied log entries), it is guaranteed
/// `applied < read_log_id`.
///
/// [`Linearizer::await_applied()`]: crate::raft::linearizable_read::Linearizer::await_applied
#[since(version = "0.10.0")]
#[derive(Debug, Clone)]
pub struct LinearizeState<C>
where C: RaftTypeConfig
{
    read_log_id: LogId<C>,
    applied: Option<LogId<C>>,
}

impl<C> fmt::Display for LinearizeState<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ready = if self.is_ready() { "ready" } else { "not_ready" };
        write!(
            f,
            "LinearizeState({ready}){{ read_log_id: {}, applied: {} }}",
            self.read_log_id,
            self.applied.display()
        )
    }
}

impl<C> LinearizeState<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(read_log_id: LogId<C>, applied: Option<LogId<C>>) -> Self {
        Self { read_log_id, applied }
    }

    /// Updates the applied log ID and returns the modified state.
    pub(crate) fn with_applied(mut self, applied: Option<LogId<C>>) -> Self {
        self.applied = applied;
        self
    }

    /// Returns whether the linearizable read is ready to be performed.
    ///
    /// This method checks if the state machine has applied enough log entries to satisfy
    /// the linearizability requirement. It returns `true` when `applied >= read_log_id`,
    /// meaning the state machine has caught up to the point where a linearizable read
    /// can be safely performed.
    #[since(version = "0.10.0")]
    pub fn is_ready(&self) -> bool {
        self.applied.as_ref() >= Some(&self.read_log_id)
    }

    /// Return the `read_log_id` of this read operation.
    ///
    /// It is the max of the current leader noop-log-id and the last committed log id.
    /// See: [`read` docs](crate::docs::protocol::read).
    #[since(version = "0.10.0")]
    pub fn read_log_id(&self) -> &LogId<C> {
        &self.read_log_id
    }

    /// The last applied log ID.
    #[since(version = "0.10.0")]
    pub fn applied(&self) -> Option<&LogId<C>> {
        self.applied.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::testing::log_id;

    #[test]
    fn test_display() {
        let state = LinearizeState::new(log_id(1, 1, 1), Some(log_id(1, 1, 0)));
        assert_eq!(
            format!("{}", state),
            "LinearizeState(not_ready){ read_log_id: T1-N1.1, applied: T1-N1.0 }"
        );

        let state = state.with_applied(Some(log_id(3, 3, 3)));
        assert_eq!(
            format!("{}", state),
            "LinearizeState(ready){ read_log_id: T1-N1.1, applied: T3-N3.3 }"
        );
    }
}
