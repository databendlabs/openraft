use super::EffectiveMembership;
use super::HardState;
use super::LogId;

/// A struct used to represent the initial state which a Raft node needs when first starting.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InitialState {
    /// The last entry.
    pub last_log_id: LogId,

    /// The LogId of the last log applied to the state machine.
    pub last_applied: LogId,

    /// The saved hard state of the node.
    pub hard_state: HardState,

    /// The latest cluster membership configuration found, in log or in state machine, else a new initial
    /// membership config consisting only of this node's ID.
    pub last_membership: EffectiveMembership,
}
