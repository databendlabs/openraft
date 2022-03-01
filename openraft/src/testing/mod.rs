mod store_builder;
mod suite;

pub use store_builder::DefensiveStoreBuilder;
pub use store_builder::StoreBuilder;
pub use suite::Suite;

crate::declare_raft_types!(
    /// Dummy Raft types for the purpose of testing internal structures requiring
    /// `RaftTypeConfig`, like `MembershipConfig`.
    pub(crate) DummyConfig: D = u64, R = u64, NodeId = u64
);
