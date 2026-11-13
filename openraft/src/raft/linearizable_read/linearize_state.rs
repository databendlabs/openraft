use std::fmt;

use display_more::DisplayOptionExt;
use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::raft::linearizable_read::ReadLogId;
use crate::type_config::alias::LogIdOf;

/// Represents the state after awaiting the applied log entries for a linearizable read.
///
/// This is returned by [`Linearizer::try_await_ready()`] after waiting for the state
/// machine to apply all necessary log entries for a linearizable read.
///
/// This struct retains the leader's no-op log ID and committed log ID from read authorization.
/// The required read log ID is their maximum. Applied progress belongs to `node_id`; it is not
/// the applied log ID sampled by a later state-machine query.
///
/// If the state is ready, it is guaranteed `applied >= read_log_id`.
/// If the state is not ready (timeout waiting for the applied log entries), it is guaranteed
/// `applied < read_log_id`.
///
/// [`Linearizer::try_await_ready()`]: crate::raft::linearizable_read::Linearizer::try_await_ready
#[since(version = "0.10.1", change = "retain no-op and committed log IDs separately")]
#[since(version = "0.10.0")]
#[derive(Debug, Clone)]
pub struct LinearizeState<C>
where C: RaftTypeConfig
{
    /// The node from which this Linearizer collects the applied log ID.
    node_id: C::NodeId,
    noop_log_id: LogIdOf<C>,
    committed: Option<LogIdOf<C>>,
    applied: Option<LogIdOf<C>>,
}

impl<C> fmt::Display for LinearizeState<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LinearizeState[id={}]", self.node_id)?;

        write!(
            f,
            "{{ read_log_id: {}, applied: {} }}",
            self.read_log_id(),
            self.applied.display()
        )
    }
}

impl<C> LinearizeState<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(
        node_id: C::NodeId,
        noop_log_id: LogIdOf<C>,
        committed: Option<LogIdOf<C>>,
        applied: Option<LogIdOf<C>>,
    ) -> Self {
        Self {
            node_id,
            noop_log_id,
            committed,
            applied,
        }
    }

    /// Updates the applied log ID and returns the modified state.
    pub(crate) fn with_applied(mut self, node_id: C::NodeId, applied: Option<LogIdOf<C>>) -> Self {
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
        self.node_id == *node_id
            && self.applied.as_ref() >= Some(&self.noop_log_id)
            && self.applied.as_ref() >= self.committed.as_ref()
    }

    /// Return the node from which the applied progress was collected.
    #[since(version = "0.10.0")]
    pub fn node_id(&self) -> &C::NodeId {
        &self.node_id
    }

    /// The no-op log ID of the leadership that authorized the read.
    #[since(version = "0.10.1")]
    pub fn noop_log_id(&self) -> &LogIdOf<C> {
        &self.noop_log_id
    }

    /// The committed log ID captured when the read was authorized.
    #[since(version = "0.10.1")]
    pub fn committed(&self) -> Option<&LogIdOf<C>> {
        self.committed.as_ref()
    }

    /// Return the `read_log_id` of this read operation.
    ///
    /// It is the maximum of the no-op and committed log IDs captured at authorization.
    /// See: [`read` docs](crate::docs::protocol::read).
    #[since(version = "0.10.1", change = "calculate and return an owned ReadLogId")]
    #[since(version = "0.10.0", change = "return ReadLogId instead of LogId")]
    #[since(version = "0.10.0")]
    pub fn read_log_id(&self) -> ReadLogId<C> {
        ReadLogId::new(self.noop_log_id.clone(), self.committed.clone())
    }

    /// The last known applied log ID on [`Self::node_id`].
    #[since(version = "0.10.0")]
    pub fn applied(&self) -> Option<&LogIdOf<C>> {
        self.applied.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::testing::UTConfig;
    use crate::engine::testing::log_id;

    #[test]
    fn test_read_log_id() {
        let noop = log_id(2, 1, 10);
        let cases = [
            (None, noop),
            (Some(log_id(1, 1, 8)), noop),
            (Some(log_id(2, 1, 9)), noop),
            (Some(noop), noop),
            (Some(log_id(2, 1, 20)), log_id(2, 1, 20)),
        ];
        for (committed, expected) in cases {
            let state: LinearizeState<UTConfig> = LinearizeState::new(1, noop, committed, None);
            assert_eq!(&noop, state.noop_log_id());
            assert_eq!(committed.as_ref(), state.committed());
            assert_eq!(&expected, state.read_log_id().log_id());
        }
    }

    #[test]
    fn test_is_ready_on_node() {
        let noop = log_id(2, 1, 10);
        let committed = log_id(2, 1, 20);
        let cases = [
            (None, false),
            (Some(noop), false),
            (Some(log_id(2, 1, 19)), false),
            (Some(committed), true),
            (Some(log_id(2, 1, 21)), true),
        ];
        for (applied, ready) in cases {
            let state: LinearizeState<UTConfig> = LinearizeState::new(1, noop, Some(committed), applied);
            assert_eq!(ready, state.is_ready_on_node(&1));
            assert!(!state.is_ready_on_node(&2));
        }
    }

    #[test]
    fn test_with_applied_preserves_read_boundaries() {
        let noop = log_id(2, 1, 10);
        let committed = log_id(2, 1, 20);
        let state: LinearizeState<UTConfig> = LinearizeState::new(1, noop, Some(committed), Some(noop));
        let state = state.with_applied(2, Some(committed));

        assert_eq!(&2, state.node_id());
        assert_eq!(&noop, state.noop_log_id());
        assert_eq!(Some(&committed), state.committed());
        assert_eq!(Some(&committed), state.applied());
        assert_eq!(&committed, state.read_log_id().log_id());
        assert!(state.is_ready_on_node(&2));
        assert!(!state.is_ready_on_node(&1));
    }

    #[test]
    fn test_display() {
        let state: LinearizeState<UTConfig> = LinearizeState::new(1, log_id(1, 1, 1), None, Some(log_id(1, 1, 0)));
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
