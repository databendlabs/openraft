//! Runs the storage test suite of `openraft::testing::log` with `HierarchicalMembership`.

use log_mem::LogStore;
use openraft::StorageError;
use openraft::testing::log::StoreBuilder;
use openraft::testing::log::Suite;
use openraft::type_config::TypeConfigExt;
use openraft_membership::HierarchicalMembership;
use sm_mem::StateMachineStore;

openraft::declare_raft_types!(
    TypeConfig:
        D = types_kv::Request,
        R = types_kv::Response,
        Node = (),
        Membership = HierarchicalMembership<u64, ()>,
        MembershipEdit = openraft_membership::HierarchicalChange<Self::NodeId, Self::Node>,
);

struct Builder;

impl StoreBuilder<TypeConfig, LogStore<TypeConfig>, StateMachineStore<TypeConfig>, ()> for Builder {
    async fn build(
        &self,
    ) -> Result<((), LogStore<TypeConfig>, StateMachineStore<TypeConfig>), StorageError<TypeConfig>> {
        Ok(((), LogStore::default(), StateMachineStore::default()))
    }
}

#[test]
fn test_suite_with_hierarchical_membership() {
    TypeConfig::run(async {
        Suite::test_all(Builder).await.unwrap();
    });
}
