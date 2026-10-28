use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::Instant;
use openraft::SnapshotPolicy;
use openraft::alias::AsyncRuntimeOf;
use openraft::alias::VoteOf;
use openraft::network::RPCOption;
use openraft::network::RaftNetworkFactory;
use openraft::network::v2::RaftNetworkV2;
use openraft::raft::VoteRequest;
use openraft::type_config::TypeConfigExt;
use openraft_membership::ThresholdMembership;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::custom_membership::TestCluster;
use crate::fixtures::custom_membership::cluster_config;
use crate::fixtures::delay_from_env;
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

/// `OPENRAFT_NETWORK_SEND_DELAY` keeps the caller default when absent and overrides it when set.
#[test]
fn delay_from_env_keeps_fallback_or_uses_decimal() {
    let absent = delay_from_env(None, 15);
    assert_eq!(15, absent);

    let present = delay_from_env(Some("40"), 15);
    assert_eq!(40, present);
}

/// One router RPC sleeps at least 1 ms and no more than the delay set after build.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn router_send_delay_bounds_one_rpc() -> Result<()> {
    let config = Config {
        enable_tick: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    router.new_raft_node(2).await?;
    router.network_send_delay(SEND_DELAY_MS);

    let mut client = RaftNetworkFactory::new_client(&mut router, 2, &()).await;
    let vote = VoteOf::<TypeConfig>::new(1, 1);
    let request = VoteRequest::new(vote, None);
    let option = RPCOption::new(Duration::from_secs(1));

    let started = TypeConfig::now();
    let _response = client.vote(request, option).await?;
    let finished = TypeConfig::now();
    let elapsed = finished.saturating_duration_since(started);
    assert_one_rpc_delay(elapsed, SEND_DELAY_MS);
    Ok(())
}

/// A custom cluster keeps the snapshot policy it was started with.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn custom_cluster_runs_snapshot_policy_never() -> Result<()> {
    let cluster = start_cluster(SnapshotPolicy::Never).await?;
    let policy = cluster.raft(1).config().snapshot_policy.clone();
    assert_eq!(SnapshotPolicy::Never, policy);
    Ok(())
}

/// One custom-membership RPC sleeps at least 1 ms and no more than the delay set after start.
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
    assert_one_rpc_delay(elapsed, SEND_DELAY_MS);
    Ok(())
}

async fn start_cluster(snapshot_policy: SnapshotPolicy) -> Result<TestCluster<DelayConfig>> {
    let voters = btreeset! {1, 2};
    let membership = ThresholdMembership::new(voters.clone(), 2, voters)?;
    let config = cluster_config(snapshot_policy);
    let cluster = TestCluster::<DelayConfig>::start(config, [1, 2], membership).await?;
    Ok(cluster)
}

fn assert_one_rpc_delay(elapsed: Duration, delay_ms: u64) {
    let elapsed_ms = elapsed.as_millis();
    let upper_ms = delay_upper_bound_ms(delay_ms);
    let within = elapsed_ms >= 1 && elapsed_ms <= upper_ms;
    assert!(within, "elapsed {elapsed_ms} ms, allowed 1..={upper_ms}");
}

fn delay_upper_bound_ms(delay_ms: u64) -> u128 {
    let delay_ms = u128::from(delay_ms);
    #[cfg(feature = "rt-sim")]
    {
        delay_ms
    }
    #[cfg(not(feature = "rt-sim"))]
    {
        // Tokio's clock runs a little past the sleep.
        const SCHEDULING_ALLOWANCE_MS: u128 = 50;
        delay_ms + SCHEDULING_ALLOWANCE_MS
    }
}
