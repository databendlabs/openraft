use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::SnapshotPolicy;
use openraft::alias::AsyncRuntimeOf;
use openraft::alias::MembershipOf;
use openraft::async_runtime::WatchReceiver;
use openraft::entry::RaftEntry;
use openraft::errors::ChangeMembershipError;
use openraft::errors::ClientWriteError;
use openraft::errors::EmptyMembership;
use openraft::errors::ErrorSource;
use openraft::errors::MembershipError;
use openraft::errors::RaftError;
use openraft::impls::BoxedErrorSource;
use openraft::membership::RaftMembership;
use openraft::membership::RaftMembershipEdit;
use openraft::testing::log_id;
use openraft_membership::ThresholdChange;
use openraft_membership::ThresholdMembership;
use openraft_membership::errors::InvalidThresholds;

use crate::fixtures::custom_membership::TestCluster;
use crate::fixtures::custom_membership::cluster_config;
use crate::fixtures::ut_harness;

openraft::declare_raft_types!(
    TypeConfig:
        D = String,
        R = (),
        Node = (),
        Membership = ThresholdMembership<u64, ()>,
        MembershipEdit = CustomEdit,
        AsyncRuntime = AsyncRuntimeOf<openraft_memstore::TypeConfig>,
);

#[derive(Debug)]
enum CustomEdit {
    Standard(ThresholdChange<u64, ()>),
    PromoteAllLearners,
    Clear,
}

impl fmt::Display for CustomEdit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl RaftMembershipEdit<MembershipOf<TypeConfig>> for CustomEdit {
    type Error = InvalidThresholds;

    fn apply(
        self,
        membership: &MembershipOf<TypeConfig>,
        retain: bool,
    ) -> Result<MembershipOf<TypeConfig>, MembershipError<u64, Self::Error>> {
        match self {
            Self::Standard(change) => change.apply(membership, retain),
            Self::PromoteAllLearners => {
                let nodes = membership.node_ids().map(|id| (id, ()));
                let nodes: BTreeMap<_, _> = nodes.collect();
                let voter_ids = nodes.keys().copied();
                let voters = voter_ids.collect();
                let write_size = nodes.len() as u64 / 2 + 1;
                let target = ThresholdMembership::new(voters, write_size, nodes)?;
                Ok(target)
            }
            Self::Clear => Ok(ThresholdMembership::default()),
        }
    }

    fn add_learner(node_id: u64, node: ()) -> Self {
        let change =
            <ThresholdChange<u64, ()> as RaftMembershipEdit<MembershipOf<TypeConfig>>>::add_learner(node_id, node);
        Self::Standard(change)
    }

    fn make_uniform() -> Self {
        let change = <ThresholdChange<u64, ()> as RaftMembershipEdit<MembershipOf<TypeConfig>>>::make_uniform();
        Self::Standard(change)
    }
}

/// A user edit operates on built-in membership and keeps learner addition and joint completion.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn custom_edit_uses_current_membership_and_completes_joint_consensus() -> Result<()> {
    let voters = btreeset! {1, 2, 3};
    let initial = MembershipOf::<TypeConfig>::new(voters.clone(), 2, voters)?;
    let cluster = TestCluster::<TypeConfig>::start(
        cluster_config(SnapshotPolicy::Never),
        btreeset! {1, 2, 3, 4},
        initial.clone(),
    )
    .await?;

    tracing::info!(
        node = 1,
        "--- elect the leader and commit its first log before applying edits"
    );
    {
        cluster.raft(1).trigger().elect(false).await?;
        cluster.raft(1).wait(timeout()).applied_index(Some(1), "commit leader blank").await?;
    }

    tracing::info!(
        node = 4,
        log_index = 2,
        "--- add a learner through the configured edit constructor"
    );
    let with_learner = {
        cluster.raft(1).add_learner(4, (), true).await?;
        let nodes = btreeset! {1, 2, 3, 4};
        MembershipOf::<TypeConfig>::new(btreeset! {1, 2, 3}, 2, nodes)?
    };

    tracing::info!(
        log_index = 4,
        "--- promote all current learners through joint consensus"
    );
    let target = {
        let response = cluster.raft(1).change_membership(CustomEdit::PromoteAllLearners, true).await?;
        let expected_log_id = log_id::<TypeConfig>(1, 1, 4);
        assert_eq!(expected_log_id, response.log_id);
        let voters = btreeset! {1, 2, 3, 4};
        let target = MembershipOf::<TypeConfig>::new(voters.clone(), 3, voters)?;
        let expected_membership = Some(target.clone());
        assert_eq!(expected_membership, response.membership);
        cluster.raft(1).wait(timeout()).applied_index(Some(4), "publish the custom target").await?;
        target
    };

    tracing::info!(
        log_index = 4,
        "--- reject an invalid custom target without appending a log"
    );
    {
        let result = cluster.raft(1).change_membership(CustomEdit::Clear, true).await;
        let actual = result.unwrap_err();
        let error = ChangeMembershipError::EmptyMembership(EmptyMembership {});
        let expected = RaftError::APIError(ClientWriteError::ChangeMembershipError(error));
        assert_eq!(expected, actual);
        let raft = cluster.raft(1);
        let metrics_rx = raft.metrics();
        let metrics = metrics_rx.borrow_watched().clone();
        let expected_last_log_index = Some(4);
        assert_eq!(expected_last_log_index, metrics.last_log_index);
        let membership = metrics.membership_config.membership();
        assert_eq!(&target, membership);
    }

    tracing::info!(
        log_index = 4,
        "--- preserve the configured error source for an invalid threshold edit"
    );
    {
        let change = ThresholdChange::write_size(5);
        let result = cluster.raft(1).change_membership(CustomEdit::Standard(change), true).await;
        let invalid = InvalidThresholds {
            voter_count: 4,
            write_size: 5,
        };
        let source = BoxedErrorSource::from_error(&invalid);
        let error = ChangeMembershipError::UserDefined(source);
        let expected = RaftError::APIError(ClientWriteError::ChangeMembershipError(error));
        let actual = result.unwrap_err();
        assert_eq!(expected, actual);
    }

    tracing::info!(
        node = 1,
        "--- verify that the applied history contains joint and uniform membership entries"
    );
    {
        let joint = with_learner.joint_with(&target);
        let expected = vec![initial, with_learner, joint, target];
        let applied = cluster.applied(1);
        let memberships = applied.iter().filter_map(|entry| entry.get_membership());
        let memberships: Vec<_> = memberships.collect();
        assert_eq!(expected, memberships);
    }
    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2_000))
}
