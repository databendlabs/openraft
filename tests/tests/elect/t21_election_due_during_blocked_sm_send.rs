use std::time::Duration;

use anyhow::Result;
use openraft::Config;
use openraft::RPCTypes;
use openraft::ServerState;
use openraft::async_runtime::OneshotSender;
use openraft::type_config::TypeConfigExt;
use openraft::type_config::alias::OneshotReceiverOf;
use openraft::type_config::alias::OneshotSenderOf;
use openraft_memstore::TypeConfig;

use crate::fixtures::MemRaft;
use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// How long a read waits before the test concludes that RaftCore is blocked.
const BACKPRESSURE_TIMEOUT: Duration = Duration::from_millis(50);

/// A follower whose election comes due while RaftCore waits on a state-machine command send
/// elects once the send completes.
///
/// The deadline notification arrives while RaftCore waits for room in the state-machine command
/// channel. The election it starts must send nothing before the send completes, and must not be
/// lost: the deadline does not change during the wait, so a dropped notification would never be
/// sent again.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn election_due_during_blocked_state_machine_send() -> Result<()> {
    let config = Config {
        state_machine_channel_size: Some(1),
        ..Default::default()
    };
    let election_timeout_max = Duration::from_millis(config.election_timeout_max);
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- create cluster of 0,1,2; node 0 becomes leader");
    let log_index = router.new_cluster([0, 1, 2], []).await?;
    let n1 = router.get_raft_handle(&1)?;

    tracing::info!(
        log_index,
        "--- disable elections on node 2, so that node 1 is the one to elect"
    );
    router.get_raft_handle(&2)?.runtime_config().elect(false);

    tracing::info!(
        log_index,
        "--- pause the state machine of node 1 and fill its only command slot"
    );
    let (release_1, release_2) = {
        let (started_1, release_1) = gate(&n1).await?;
        started_1.await?;
        let (_started_2, release_2) = gate(&n1).await?;
        (release_1, release_2)
    };

    tracing::info!(
        log_index,
        "--- isolate the leader, then block node 1 on the send of a snapshot command"
    );
    {
        router.set_network_error(0, true);
        n1.trigger().snapshot().await?;

        let read = n1.with_raft_state(|_| ());
        let blocked = TypeConfig::timeout(BACKPRESSURE_TIMEOUT, read).await.is_err();
        assert!(blocked, "node 1 must wait on the state-machine command send");
    }

    tracing::info!(
        log_index,
        "--- the election comes due during the send; its requests wait for the send"
    );
    {
        let votes_before = router.get_rpc_count().get(&RPCTypes::Vote).copied().unwrap_or(0);
        TypeConfig::sleep(election_timeout_max * 5).await;
        let votes_after = router.get_rpc_count().get(&RPCTypes::Vote).copied().unwrap_or(0);
        assert_eq!(votes_before, votes_after, "no vote request is sent during the send");
    }

    tracing::info!(log_index, "--- release the state machine; node 1 elects itself");
    {
        drop(release_1);
        drop(release_2);
        router.wait(&1).state(ServerState::Leader, "node 1 elected").await?;
    }

    Ok(())
}

/// Submit a state-machine function that blocks the worker until the returned sender is dropped.
///
/// The returned receiver completes once the worker runs the function.
async fn gate(n: &MemRaft) -> Result<(OneshotReceiverOf<TypeConfig, ()>, OneshotSenderOf<TypeConfig, ()>)> {
    let (started_tx, started_rx) = TypeConfig::oneshot();
    let (release_tx, release_rx) = TypeConfig::oneshot::<()>();
    n.external_state_machine_request(move |_sm| {
        Box::pin(async move {
            started_tx.send(()).ok();
            release_rx.await.ok();
        })
    })
    .await?;
    Ok((started_rx, release_tx))
}
