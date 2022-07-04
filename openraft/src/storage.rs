use crate::types::v065::EffectiveMembership;
pub use crate::types::v065::HardState;
pub use crate::types::v065::InitialState;
use crate::types::v065::LogId;
use crate::types::v065::Membership;
use crate::types::v065::NodeId;
pub use crate::types::v065::RaftStorage;
pub use crate::types::v065::Snapshot;

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
