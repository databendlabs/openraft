mod store_builder;
mod suite;

pub use store_builder::DefensiveStoreBuilder;
pub use store_builder::StoreBuilder;
pub use suite::Suite;

use crate::DummyNetwork;
use crate::DummyStorage;

crate::declare_raft_types!(
    /// Dummy Raft types for the purpose of testing internal structures requiring
    /// `RaftTypeConfig`, like `MembershipConfig`.
    pub(crate) DummyConfig: D = u64, R = u64, S = DummyStorage<Self>, N = DummyNetwork<Self>, NodeId = u64
);
