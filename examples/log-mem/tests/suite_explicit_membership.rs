//! Runs the storage test suite of `openraft::testing::log` with `ExplicitMembership`.

use log_mem::LogStore;
use openraft::StorageError;
use openraft::testing::log::StoreBuilder;
use openraft::testing::log::Suite;
use openraft::type_config::TypeConfigExt;
use openraft_membership::ExplicitMembership;
use sm_mem::StateMachineStore;

openraft::declare_raft_types!(
    TypeConfig:
        D = types_kv::Request,
        R = types_kv::Response,
        Node = (),
        Membership = ExplicitMembership<u64, ()>,
        MembershipEdit = openraft_membership::ExplicitChange<Self::NodeId, Self::Node>,
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
fn test_suite_with_explicit_membership() {
    TypeConfig::run(async {
        Suite::test_all(Builder).await.unwrap();
    });
}
