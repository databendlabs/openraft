use crate::EffectiveMembership;
pub use crate::HardState;
pub use crate::InitialState;
use crate::LogId;
use crate::Membership;
use crate::NodeId;
pub use crate::RaftStorage;
pub use crate::Snapshot;

impl InitialState {
    /// Create a new instance for a pristine Raft node.
    ///
    /// ### `id`
    /// The ID of the Raft node.
    pub fn new_initial(id: NodeId) -> Self {
        Self {
            last_log_id: LogId { term: 0, index: 0 },
            last_applied: LogId { term: 0, index: 0 },
            hard_state: HardState {
                current_term: 0,
                voted_for: None,
            },
            last_membership: EffectiveMembership {
                log_id: LogId { term: 0, index: 0 },
                membership: Membership::new_initial(id),
            },
        }
    }
}
