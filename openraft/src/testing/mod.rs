mod store_builder;
mod suite;

pub use store_builder::DefensiveStoreBuilder;
pub use store_builder::StoreBuilder;
pub use suite::Suite;

use crate::BasicNode;
use crate::CommittedLeaderId;
use crate::LogId;

crate::declare_raft_types!(
    /// Dummy Raft types for the purpose of testing internal structures requiring
    /// `RaftTypeConfig`, like `MembershipConfig`.
    pub(crate) DummyConfig: D = u64, R = u64, NodeId = u64, Node = BasicNode
);

pub fn log_id(term: u64, index: u64) -> LogId<u64> {
    LogId::<u64> {
        leader_id: CommittedLeaderId::new(term, 1),
        index,
    }
}
