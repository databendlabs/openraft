use std::fmt;

use openraft_macros::since;

use crate::LogId;
use crate::RaftTypeConfig;
use crate::display_ext::DisplayOptionExt;

/// Represents the state after awaiting the applied log entries for a linearizable read.
///
/// This is returned by [`Linearizer::try_await_ready()`] after waiting for the state
/// machine to apply all necessary log entries for a linearizable read.
///
/// This struct contains the log IDs that were used to ensure linearizability:
/// - `read_log_id`: the log ID that was required to be applied before reading
/// - `applied`: the actual log ID that was applied to satisfy the linearizability requirement
///
/// If the state is ready, it is guaranteed `applied >= read_log_id`.
/// If the state is not ready (timeout waiting for the applied log entries), it is guaranteed
/// `applied < read_log_id`.
///
/// [`Linearizer::try_await_ready()`]: crate::raft::linearizable_read::Linearizer::try_await_ready
#[since(version = "0.10.0")]
#[derive(Debug, Clone)]
pub struct LinearizeState<C>
where C: RaftTypeConfig
{
    /// The node from which this Linearizer collects the applied log ID.
    node_id: C::NodeId,
    read_log_id: LogId<C>,
    applied: Option<LogId<C>>,
}

impl<C> fmt::Display for LinearizeState<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LinearizeState[id={}]", self.node_id)?;

        write!(
            f,
            "{{ read_log_id: {}, applied: {} }}",
            self.read_log_id,
            self.applied.display()
        )
    }
}

impl<C> LinearizeState<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(node_id: C::NodeId, read_log_id: LogId<C>, applied: Option<LogId<C>>) -> Self {
        Self {
            node_id,
            read_log_id,
            applied,
        }
    }

    /// Updates the applied log ID and returns the modified state.
    pub(crate) fn with_applied(mut self, node_id: C::NodeId, applied: Option<LogId<C>>) -> Self {
        self.node_id = node_id;
        self.applied = applied;
        self
    }

    /// Returns whether the linearizable read is ready to be performed.
    ///
    /// This method checks if the state machine has applied enough log entries to satisfy
    /// the linearizability requirement. It returns `true` when `applied >= read_log_id`,
    /// meaning the state machine has caught up to the point where a linearizable read
    /// can be safely performed.
    ///
    /// If the local_node_id is different, the `applied` is unknown.
    pub(crate) fn is_ready_on_node(&self, node_id: &C::NodeId) -> bool {
        self.node_id == *node_id && self.applied.as_ref() >= Some(&self.read_log_id)
    }

    /// Return the node id on which the linearizer is created.
    #[since(version = "0.10.0")]
    pub fn node_id(&self) -> &C::NodeId {
        &self.node_id
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
        let state = LinearizeState::new(1, log_id(1, 1, 1), Some(log_id(1, 1, 0)));
        assert_eq!(
            format!("{}", state),
            "LinearizeState[id=1]{ read_log_id: T1-N1.1, applied: T1-N1.0 }"
        );

        let state = state.with_applied(2, Some(log_id(3, 3, 3)));
        assert_eq!(
            format!("{}", state),
            "LinearizeState[id=2]{ read_log_id: T1-N1.1, applied: T3-N3.3 }"
        );
    }
}
