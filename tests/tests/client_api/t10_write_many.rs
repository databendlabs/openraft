use anyhow::Result;
use openraft::Config;
use openraft::errors::ClientWriteError;
use openraft::errors::ForwardReason;
use openraft::impls::ProgressResponder;
use openraft::raft::WriteOption;
use openraft::type_config::alias::LeaderIdOf;
use openraft::vote::RaftLeaderId;
use openraft_memstore::ClientRequest;
use openraft_memstore::ClientResponse;
use openraft_memstore::IntoMemClientRequest;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// `Raft::write_many()` appends every write in order at consecutive log indexes, and reports each
/// result to the write's own responder.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn write_many() -> Result<()> {
    let config = Config {
        enable_tick: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initialize a 3-node cluster");
    let mut log_index = router.new_cluster([0, 1, 2], []).await?;
    let n0 = router.get_raft_handle(&0)?;

    tracing::info!(
        log_index,
        "--- build one batch with write_many(), push() and extend(); one write has no responder"
    );
    {
        let (responder0, rx0) = ProgressResponder::complete_only();
        let (responder2, rx2) = ProgressResponder::complete_only();
        let (responder3, rx3) = ProgressResponder::complete_only();

        n0.write_many([
            (ClientRequest::make_request("foo", 0), Some(responder0)),
            (ClientRequest::make_request("foo", 1), None),
        ])
        .push(ClientRequest::make_request("foo", 2), Some(responder2))
        .extend([(ClientRequest::make_request("foo", 3), Some(responder3))])
        .await?;

        // The state machine returns the previous status of the client, which shows the apply order.
        let got = rx0.await??;
        assert_eq!(&log_id(1, 0, log_index + 1), got.log_id());
        assert_eq!(&ClientResponse(None), got.response());

        let got = rx2.await??;
        assert_eq!(&log_id(1, 0, log_index + 3), got.log_id());
        assert_eq!(&ClientResponse(Some("request-1".to_string())), got.response());

        let got = rx3.await??;
        assert_eq!(&log_id(1, 0, log_index + 4), got.log_id());
        assert_eq!(&ClientResponse(Some("request-2".to_string())), got.response());

        log_index += 4;
        router.wait_applied_at_least([0, 1, 2], log_index, "batch applied on every node").await?;
    }

    Ok(())
}

/// A batch with an expected leader is appended as a whole when the leader matches, and rejected
/// as a whole when it does not; `with_option()` replaces the option set by `with_leader()`.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn write_many_with_leader() -> Result<()> {
    let config = Config {
        enable_tick: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initialize a 3-node cluster");
    let mut log_index = router.new_cluster([0, 1, 2], []).await?;
    let n0 = router.get_raft_handle(&0)?;

    let leader_id = n0.as_leader()?.to_committed_leader_id();
    let fake_leader = LeaderIdOf::<TypeConfig>::new(100, 99).to_committed();

    tracing::info!(log_index, "--- a batch expecting the current leader is appended");
    {
        let (responder1, rx1) = ProgressResponder::complete_only();
        let (responder2, rx2) = ProgressResponder::complete_only();

        n0.write_many([
            (ClientRequest::make_request("foo", 1), Some(responder1)),
            (ClientRequest::make_request("foo", 2), Some(responder2)),
        ])
        .with_leader(leader_id)
        .await?;

        assert_eq!(&log_id(1, 0, log_index + 1), rx1.await??.log_id());
        assert_eq!(&log_id(1, 0, log_index + 2), rx2.await??.log_id());
        log_index += 2;
    }

    tracing::info!(
        log_index,
        "--- a batch expecting another leader fails every write, through with_leader() or with_option()"
    );
    {
        let (responder3, rx3) = ProgressResponder::complete_only();
        let (responder4, rx4) = ProgressResponder::complete_only();
        n0.write_many([
            (ClientRequest::make_request("foo", 3), Some(responder3)),
            (ClientRequest::make_request("foo", 4), Some(responder4)),
        ])
        .with_leader(fake_leader)
        .await?;

        let (responder5, rx5) = ProgressResponder::complete_only();
        n0.write_many([(ClientRequest::make_request("foo", 5), Some(responder5))])
            .with_option(WriteOption::default().with_leader(fake_leader))
            .await?;

        for rx in [rx3, rx4, rx5] {
            let err = rx.await?.unwrap_err();
            let ClientWriteError::ForwardToLeader(forward) = err else {
                panic!("expected ForwardToLeader, got: {:?}", err)
            };
            assert_eq!(Some(0), forward.leader_id);
            assert_eq!(ForwardReason::NotLeader, forward.reason);
        }
    }

    tracing::info!(
        log_index,
        "--- with_option() drops the expected leader set before it; the next write follows request-2"
    );
    {
        let (responder6, rx6) = ProgressResponder::complete_only();
        n0.write_many([(ClientRequest::make_request("foo", 6), Some(responder6))])
            .with_leader(fake_leader)
            .with_option(WriteOption::default())
            .await?;

        let got = rx6.await??;
        assert_eq!(&log_id(1, 0, log_index + 1), got.log_id());
        assert_eq!(&ClientResponse(Some("request-2".to_string())), got.response());
        log_index += 1;

        router.wait_applied_at_least([0, 1, 2], log_index, "rejected writes are not applied").await?;
    }

    Ok(())
}
