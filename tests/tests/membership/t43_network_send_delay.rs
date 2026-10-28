use anyhow::Result;
use maplit::btreeset;
use openraft::Instant;
use openraft::SnapshotPolicy;
use openraft::alias::AsyncRuntimeOf;
use openraft::alias::VoteOf;
use openraft::raft::VoteRequest;
use openraft::type_config::TypeConfigExt;
use openraft_membership::ThresholdMembership;

use crate::fixtures::custom_membership::TestCluster;
use crate::fixtures::custom_membership::cluster_config;
use crate::fixtures::ut_harness;

const SEND_DELAY_MS: u64 = 50;

openraft::declare_raft_types!(
    DelayConfig:
        D = String,
        R = (),
        Node = (),
        Membership = ThresholdMembership<u64, ()>,
        MembershipEdit = openraft_membership::ThresholdChange<Self::NodeId, Self::Node>,
        AsyncRuntime = AsyncRuntimeOf<openraft_memstore::TypeConfig>,
);

/// A custom cluster keeps the snapshot policy it was started with.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn custom_cluster_runs_snapshot_policy_never() -> Result<()> {
    let cluster = start_cluster(SnapshotPolicy::Never).await?;
    let policy = cluster.raft(1).config().snapshot_policy.clone();
    assert_eq!(SnapshotPolicy::Never, policy);
    Ok(())
}

/// One custom-membership RPC observes the send delay set after start.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn custom_membership_send_delay_bounds_one_rpc() -> Result<()> {
    let cluster = start_cluster(SnapshotPolicy::Never).await?;
    cluster.network_send_delay(SEND_DELAY_MS);

    let vote = VoteOf::<DelayConfig>::new(1, 1);
    let request = VoteRequest::new(vote, None);
    let started = DelayConfig::now();
    let _response = cluster.send_vote(1, 2, request).await?;
    let finished = DelayConfig::now();
    let elapsed = finished.saturating_duration_since(started);
    let elapsed_ms = elapsed.as_millis();
    assert!(elapsed_ms >= 1, "elapsed {elapsed_ms} ms, expected at least 1");

    if cfg!(feature = "rt-sim") {
        let upper_ms = u128::from(SEND_DELAY_MS);
        assert!(
            elapsed_ms <= upper_ms,
            "elapsed {elapsed_ms} ms, expected at most {upper_ms}"
        );
    }
    Ok(())
}

async fn start_cluster(snapshot_policy: SnapshotPolicy) -> Result<TestCluster<DelayConfig>> {
    let voters = btreeset! {1, 2};
    let membership = ThresholdMembership::new(voters.clone(), 2, voters)?;
    let config = cluster_config(snapshot_policy);
    let cluster = TestCluster::<DelayConfig>::start(config, [1, 2], membership).await?;
    Ok(cluster)
}
