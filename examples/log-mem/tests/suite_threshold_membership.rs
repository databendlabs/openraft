//! Runs the storage test suite of `openraft::testing::log` with a membership type other than the
//! default `Membership`.

use log_mem::LogStore;
use openraft::StorageError;
use openraft::membership::ThresholdMembership;
use openraft::testing::log::StoreBuilder;
use openraft::testing::log::Suite;
use openraft::type_config::TypeConfigExt;
use sm_mem::StateMachineStore;

openraft::declare_raft_types!(
    TypeConfig:
        D = types_kv::Request,
        R = types_kv::Response,
        Node = (),
        Membership = ThresholdMembership<u64, ()>,
        MembershipEdit = openraft::impls::ThresholdChange<Self::NodeId, Self::Node>,
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
fn test_suite_with_threshold_membership() {
    TypeConfig::run(async {
        Suite::test_all(Builder).await.unwrap();
    });
}
