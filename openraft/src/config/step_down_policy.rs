//! Policy for stepping down a Leader that is removed from the membership config.

use openraft_macros::since;

/// Policy for stepping down a Leader that is removed from a committed membership config.
///
/// It is the value of
/// [`Config::removed_leader_step_down`](crate::Config::removed_leader_step_down).
#[since(version = "0.10.0")]
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum StepDownPolicy {
    /// Never step down automatically.
    ///
    /// The removed Leader keeps leading, until the application reverts it to a learner with
    /// [`Trigger::refresh_server_state()`].
    ///
    /// [`Trigger::refresh_server_state()`]: crate::raft::trigger::Trigger::refresh_server_state
    Never,

    /// Wait the specified number of milliseconds before attempting a leadership transfer.
    ///
    /// The delay starts when the membership config that removes the Leader is committed. When
    /// the delay expires, if the node is still a removed Leader, it attempts to transfer leadership
    /// to the most up-to-date voter, if one is available.
    ///
    /// Whether or not there is a transfer target, it waits one more
    /// [`heartbeat_interval`](crate::Config::heartbeat_interval) before requesting a fallback role
    /// refresh to become a learner. The refresh does nothing if the vote or effective membership
    /// has changed since the removal was rechecked.
    ///
    /// `After(0)` skips the initial delay, not the heartbeat interval before the fallback.
    After(u64),
}
