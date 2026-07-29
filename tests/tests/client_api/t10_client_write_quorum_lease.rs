use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::ServerState;
use openraft::async_runtime::WatchReceiver;
use openraft::errors::ClientWriteError;
use openraft::errors::ForwardToLeader;
use openraft::errors::RaftError;
use openraft::impls::ProgressResponder;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::ClientRequest;
use openraft_memstore::IntoMemClientRequest;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// A leader with an expired quorum lease rejects new writes without abandoning pending writes or
/// its ability to recover the lease.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn client_write_requires_valid_quorum_lease() -> Result<()> {
    let config = Arc::new(
        Config {
            heartbeat_interval: 20,
            election_timeout_min: 100,
            election_timeout_max: 200,
            enable_tick: false,
            enable_heartbeat: false,
            enable_elect: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;
    let n0 = router.get_raft_handle(&0)?;

    router.set_unreachable(1, true);
    router.set_unreachable(2, true);

    let (responder, pending_rx) = ProgressResponder::complete_only();
    n0.client_write_ff(ClientRequest::make_request("pending", 1), Some(responder)).await?;
    log_index += 1;
    n0.wait(timeout()).log_index(Some(log_index), "pending write appended").await?;

    TypeConfig::sleep(Duration::from_millis(config.election_timeout_max)).await;

    let rejected = TypeConfig::timeout(
        Duration::from_millis(100),
        n0.client_write(ClientRequest::make_request("rejected", 2)),
    )
    .await
    .expect("an expired leader lease rejects a new write")
    .unwrap_err();

    assert_eq!(
        RaftError::APIError(ClientWriteError::ForwardToLeader(ForwardToLeader::empty())),
        rejected
    );

    let metrics = n0.metrics().borrow_watched().clone();
    assert_eq!(ServerState::Leader, metrics.state);
    assert_eq!(Some(0), metrics.current_leader);
    assert_eq!(Some(log_index), metrics.last_log_index);

    router.set_unreachable(1, false);
    router.set_unreachable(2, false);

    let old_acked = metrics.last_quorum_acked.unwrap().into_inner();
    n0.trigger().heartbeat().await?;
    n0.wait(timeout())
        .metrics(
            |m| m.last_quorum_acked.is_some_and(|acked| acked.into_inner() > old_acked),
            "leader lease recovered",
        )
        .await?;

    let recovered = n0.client_write(ClientRequest::make_request("recovered", 3)).await?;
    log_index += 1;
    assert_eq!(log_id(1, 0, log_index), recovered.log_id);

    let pending = pending_rx.await??;
    assert_eq!(log_id(1, 0, log_index - 1), pending.log_id);

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_secs(3))
}
