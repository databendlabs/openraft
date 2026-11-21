use std::time::Duration;

use anyhow::Result;
use futures::prelude::*;
use openraft::Config;
use openraft::ServerState;
use openraft::async_runtime::WatchReceiver;
use openraft::errors::ClientWriteError;
use openraft::errors::ForwardReason;
use openraft::errors::PreconditionFailed;
use openraft::impls::ProgressResponder;
use openraft::raft::ClientWriteResult;
use openraft::raft::WriteOption;
use openraft::type_config::TypeConfigExt;
use openraft::type_config::alias::LogIdOf;
use openraft_memstore::ClientRequest;
use openraft_memstore::ClientResponse;
use openraft_memstore::IntoMemClientRequest;
use openraft_memstore::TypeConfig;

use crate::fixtures::MemRaft;
use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::timeout;
use crate::fixtures::ut_harness;

/// Send one write of client `foo` that expects `last_log_id`, and wait for its result.
async fn write_expecting(
    raft: &MemRaft,
    serial: u64,
    last_log_id: LogIdOf<TypeConfig>,
) -> Result<ClientWriteResult<TypeConfig>> {
    let (responder, rx) = ProgressResponder::complete_only();
    raft.write(ClientRequest::make_request("foo", serial))
        .responder(responder)
        .with_option(WriteOption::expect_last_log_id(last_log_id))
        .await?;
    Ok(rx.await?)
}

fn last_log_id_mismatch(expected: LogIdOf<TypeConfig>, actual: LogIdOf<TypeConfig>) -> ClientWriteError<TypeConfig> {
    ClientWriteError::PreconditionFailed(PreconditionFailed::LastLogIdMismatch {
        expected: Some(expected),
        actual: Some(actual),
    })
}

/// A write refused by an expired leader lease never enters the log, so a write that expects the
/// refused write's log id is rejected after the lease is renewed in the same term.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn write_expecting_a_refused_write_is_rejected() -> Result<()> {
    let config = Config {
        heartbeat_interval: 20,
        election_timeout_min: 100,
        election_timeout_max: 200,
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config.clone())?;

    tracing::info!("--- initialize a 3-node cluster");
    let log_index = router.new_cluster([0, 1, 2], []).await?;
    let n0 = router.get_raft_handle(&0)?;

    let last = log_id(1, 0, log_index);
    let refused = log_id(1, 0, log_index + 1);

    tracing::info!(
        log_index,
        "--- let the leader lease expire while no quorum can renew it"
    );
    {
        router.set_unreachable(1, true);
        router.set_unreachable(2, true);
        TypeConfig::sleep(Duration::from_millis(config.election_timeout_max)).await;
    }

    tracing::info!(%last, "--- the expired lease refuses the first write, which expects the last log id");
    {
        let err = write_expecting(&n0, 1, last).await?.unwrap_err();
        let ClientWriteError::ForwardToLeader(forward) = err else {
            panic!("expected ForwardToLeader, got: {:?}", err);
        };
        assert_eq!(ForwardReason::LeaseExpired, forward.reason);
    }

    tracing::info!("--- renew the lease in the same term");
    {
        router.set_unreachable(1, false);
        router.set_unreachable(2, false);

        let refresh_started = TypeConfig::now();
        n0.trigger().heartbeat().await?;
        n0.wait(timeout()).leader_with_quorum_acked(Some(refresh_started), "leader lease renewed").await?;
    }

    tracing::info!(%refused, "--- the second write expects the refused write's log id and is rejected");
    {
        let err = write_expecting(&n0, 2, refused).await?.unwrap_err();
        assert_eq!(last_log_id_mismatch(refused, last), err);

        let metrics = n0.metrics().borrow_watched().clone();
        assert_eq!(ServerState::Leader, metrics.state);
        assert_eq!(1, metrics.current_term);
        assert_eq!(Some(log_index), metrics.last_log_index);
    }

    tracing::info!(%last, "--- a write that expects the actual last log id is appended, after no other write");
    {
        let got = write_expecting(&n0, 3, last).await??;
        assert_eq!(&refused, got.log_id());
        assert_eq!(&ClientResponse(None), got.response());
    }

    Ok(())
}

/// Batches sent back to back, each expecting the last entry of the batch before it, are appended
/// in order. A write whose expected entry is no longer the last one, because another write, a
/// blank or a membership entry follows it, is rejected.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn write_expect_last_log_id() -> Result<()> {
    let config = Config {
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initialize a 3-node cluster");
    let mut log_index = router.new_cluster([0, 1, 2], []).await?;
    let n0 = router.get_raft_handle(&0)?;

    tracing::info!(
        log_index,
        "--- send three chained batches before any of them is answered"
    );
    {
        // (serials of the batch, the index of the entry it expects)
        let batches = [(1..=2, log_index), (3..=5, log_index + 2), (6..=6, log_index + 5)];

        let mut streams = Vec::new();
        for (serials, last_index) in batches {
            let requests = serials.map(|serial| ClientRequest::make_request("foo", serial));
            let write_option = WriteOption::expect_last_log_id(log_id(1, 0, last_index));
            streams.push(n0.client_write_many(requests, write_option).await?);
        }

        for stream in streams {
            let results: Vec<_> = stream.try_collect().await?;
            for result in results {
                log_index += 1;
                assert_eq!(log_id(1, 0, log_index), result?.log_id);
            }
        }
    }

    tracing::info!(
        log_index,
        "--- of two writes that expect the same last log id, sent back to back, only the first is appended"
    );
    {
        let last = log_id(1, 0, log_index);
        let write_option = WriteOption::expect_last_log_id(last);

        let first = n0.client_write_many([ClientRequest::make_request("foo", 7)], write_option.clone()).await?;
        let second = n0.client_write_many([ClientRequest::make_request("foo", 8)], write_option).await?;

        let results: Vec<_> = first.try_collect().await?;
        log_index += 1;
        assert_eq!(1, results.len());
        let appended = results.into_iter().next().unwrap()?;
        assert_eq!(log_id(1, 0, log_index), appended.log_id);

        let results: Vec<_> = second.try_collect().await?;
        assert_eq!(1, results.len());
        let Some(Err(err)) = results.into_iter().next() else {
            panic!("the second write must be rejected");
        };
        assert_eq!(last_log_id_mismatch(last, appended.log_id), err);
    }

    tracing::info!(log_index, "--- a blank entry follows the expected entry");
    {
        let expected = log_id(1, 0, log_index);

        let blank = n0.write_blank().await?;
        log_index += 1;
        assert_eq!(log_id(1, 0, log_index), blank.log_id);

        let err = write_expecting(&n0, 9, expected).await?.unwrap_err();
        assert_eq!(last_log_id_mismatch(expected, blank.log_id), err);
    }

    tracing::info!(log_index, "--- a membership entry follows the expected entry");
    {
        let expected = log_id(1, 0, log_index);

        router.new_raft_node(3).await?;
        let membership = router.add_learner(0, 3).await?;
        log_index += 1;
        assert_eq!(log_id(1, 0, log_index), membership.log_id);

        let err = write_expecting(&n0, 10, expected).await?.unwrap_err();
        assert_eq!(last_log_id_mismatch(expected, membership.log_id), err);
    }

    tracing::info!(
        log_index,
        "--- a write that expects the membership entry is appended; it follows request-7, so no rejected write was applied"
    );
    {
        let got = write_expecting(&n0, 11, log_id(1, 0, log_index)).await??;
        log_index += 1;
        assert_eq!(&log_id(1, 0, log_index), got.log_id());
        assert_eq!(&ClientResponse(Some("request-7".to_string())), got.response());

        router.wait_applied_at_least([0, 1, 2, 3], log_index, "writes applied").await?;
    }

    Ok(())
}

/// A last log id from one leader never matches on the next one, whose log ends with its own blank
/// entry.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn write_expect_last_log_id_after_leader_change() -> Result<()> {
    let config = Config {
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initialize a 3-node cluster");
    let log_index = router.new_cluster([0, 1, 2], []).await?;
    let n0 = router.get_raft_handle(&0)?;
    let n1 = router.get_raft_handle(&1)?;

    let old_last = log_id(1, 0, log_index);

    tracing::info!(
        log_index,
        "--- transfer leadership to node 1, which appends its blank entry"
    );
    let new_last = {
        n0.trigger().transfer_leader(1).await?;
        n1.wait(timeout())
            .metrics(
                |m| m.state == ServerState::Leader && m.last_applied.as_ref().map(|l| l.index()) == Some(log_index + 1),
                "node 1 leads and applied its blank entry",
            )
            .await?;
        n0.wait(timeout()).current_leader(1, "node 0 follows node 1").await?;

        let new_last = n1.data_metrics().borrow_watched().last_log.unwrap();
        assert_eq!(log_index + 1, new_last.index());
        new_last
    };

    tracing::info!(%old_last, "--- the old leader forwards a write that expects its last log id");
    {
        let err = write_expecting(&n0, 1, old_last).await?.unwrap_err();
        let ClientWriteError::ForwardToLeader(forward) = err else {
            panic!("expected ForwardToLeader, got: {:?}", err);
        };
        assert_eq!(Some(1), forward.leader_id);
    }

    tracing::info!(%old_last, %new_last, "--- the new leader rejects the same write");
    {
        let err = write_expecting(&n1, 2, old_last).await?.unwrap_err();
        assert_eq!(last_log_id_mismatch(old_last, new_last), err);
    }

    tracing::info!(%new_last, "--- a write that expects the new leader's blank entry is appended");
    {
        let got = write_expecting(&n1, 3, new_last).await??;
        assert_eq!(
            &LogIdOf::<TypeConfig>::new(new_last.leader_id, new_last.index() + 1),
            got.log_id()
        );
        assert_eq!(&ClientResponse(None), got.response());
    }

    Ok(())
}
