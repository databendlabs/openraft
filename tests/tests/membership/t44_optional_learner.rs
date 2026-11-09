use std::collections::BTreeSet;
use std::convert::Infallible;
use std::fmt;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::SnapshotPolicy;
use openraft::alias::AsyncRuntimeOf;
use openraft::async_runtime::WatchReceiver;
use openraft::errors::MembershipError;
use openraft::impls::ChangeMembers;
use openraft::impls::Membership;
use openraft::membership::RaftAddLearner;
use openraft::membership::RaftMembershipEdit;

use crate::fixtures::custom_membership::TestCluster;
use crate::fixtures::custom_membership::cluster_config;
use crate::fixtures::ut_harness;

openraft::declare_raft_types!(
    TypeConfig:
        D = String,
        R = (),
        Node = (),
        MembershipEdit = LearnerEdit,
        AsyncRuntime = AsyncRuntimeOf<openraft_memstore::TypeConfig>,
);

#[derive(Debug)]
struct LearnerEdit {
    id: u64,
    node: (),
}

impl fmt::Display for LearnerEdit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "add learner {}", self.id)
    }
}

impl RaftMembershipEdit<Membership<u64, ()>> for LearnerEdit {
    type Error = Infallible;

    fn apply(
        self,
        current: &Membership<u64, ()>,
        retain: bool,
    ) -> Result<Membership<u64, ()>, MembershipError<u64, Infallible>> {
        let change = ChangeMembers::AddNodes(std::collections::BTreeMap::from([(self.id, self.node)]));
        change.apply(current, retain)
    }
}

impl RaftAddLearner<Membership<u64, ()>> for LearnerEdit {
    fn add_learner(id: u64, node: ()) -> Self {
        Self { id, node }
    }
}

openraft::declare_raft_types!(
    NoLearnerConfig:
        D = String,
        R = (),
        Node = (),
        MembershipEdit = VotersOnly,
        AsyncRuntime = AsyncRuntimeOf<openraft_memstore::TypeConfig>,
);

#[derive(Debug)]
struct VotersOnly(BTreeSet<u64>);

impl fmt::Display for VotersOnly {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "voters {:?}", self.0)
    }
}

impl RaftMembershipEdit<Membership<u64, ()>> for VotersOnly {
    type Error = Infallible;

    fn apply(
        self,
        current: &Membership<u64, ()>,
        retain: bool,
    ) -> Result<Membership<u64, ()>, MembershipError<u64, Infallible>> {
        ChangeMembers::ReplaceAllVoters(self.0).apply(current, retain)
    }
}

/// General membership changes work without the optional learner capability.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn edit_without_learner_capability_changes_membership() -> Result<()> {
    let initial = Membership::new_with_defaults(vec![btreeset! {1}], [2]);
    let config = cluster_config(SnapshotPolicy::Never);
    let cluster = TestCluster::<NoLearnerConfig>::start(config, [1, 2], initial).await?;
    let raft = cluster.raft(1);
    let timeout = Some(Duration::from_secs(5));

    tracing::info!(
        node = 1,
        "--- commit a leader log using an edit with no learner constructor"
    );
    {
        raft.trigger().elect(false).await?;
        raft.wait(timeout).applied_index(Some(1), "commit leader blank").await?;
        cluster.raft(2).wait(timeout).applied_index(Some(1), "existing learner catches up").await?;
    }

    tracing::info!(
        log_index = 2,
        "--- promote the existing learner with a general membership edit"
    );
    {
        let response = raft.change_membership(VotersOnly(btreeset! {1, 2}), true).await?;
        assert_eq!(2, response.log_id.index());
        let expected = Membership::new_with_defaults(vec![btreeset! {1, 2}], []);
        assert_eq!(Some(expected), response.membership);
    }

    Ok(())
}

/// A learner-only edit needs no conversion from the full ChangeMembers operation set.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn optional_learner_api_preserves_both_modes_and_self_add() -> Result<()> {
    let mut config = cluster_config(SnapshotPolicy::Never);
    config.replication_lag_threshold = 0;
    let initial = Membership::new_with_defaults(vec![btreeset! {1}], []);
    let cluster = TestCluster::<TypeConfig>::start(config, [1, 2, 3], initial).await?;
    let raft = cluster.raft(1);
    let timeout = Some(Duration::from_secs(5));

    tracing::info!(
        node = 1,
        "--- elect the leader before using the optional learner capability"
    );
    {
        raft.trigger().elect(false).await?;
        raft.wait(timeout).applied_index(Some(1), "commit leader blank").await?;
    }

    tracing::info!(
        node = 2,
        log_index = 2,
        "--- add a learner without blocking on catch-up"
    );
    {
        let response = raft.add_learner(2, (), false).await?;
        assert_eq!(2, response.log_id.index());
        let expected = Membership::new_with_defaults(vec![btreeset! {1}], [2]);
        assert_eq!(Some(expected), response.membership);
        cluster.raft(2).wait(timeout).applied_index(Some(2), "non-blocking learner catches up").await?;
    }

    tracing::info!(
        node = 3,
        log_index = 3,
        "--- require blocking addition to observe learner replication"
    );
    {
        let response = raft.add_learner(3, (), true).await?;
        assert_eq!(3, response.log_id.index());
        let expected = Membership::new_with_defaults(vec![btreeset! {1}], [2, 3]);
        assert_eq!(Some(expected), response.membership);
        let metrics = raft.metrics().borrow_watched().clone();
        let replication = metrics.replication.as_ref().expect("the node remains leader");
        let matched = replication.get(&3).expect("blocking addition observes learner replication");
        assert!(matched.as_ref() >= Some(&response.log_id));
    }

    tracing::info!(
        node = 1,
        log_index = 4,
        "--- self-add returns without demoting the leader"
    );
    {
        let response = raft.add_learner(1, (), true).await?;
        assert_eq!(4, response.log_id.index());
        let expected = Membership::new_with_defaults(vec![btreeset! {1}], [2, 3]);
        assert_eq!(Some(expected), response.membership);
    }

    Ok(())
}
