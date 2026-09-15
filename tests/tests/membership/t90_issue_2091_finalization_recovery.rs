use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::EntryPayload;
use openraft::LogIdOptionExt;
use openraft::Membership;
use openraft::RPCTypes;
use openraft::RaftLogReader;
use openraft::ServerState;
use openraft::Vote;
use openraft::async_runtime::OneshotSender;
use openraft::errors::NetworkError;
use openraft::errors::RPCError;
use openraft::raft::FlushPoint;
use openraft::storage::RaftLogStorage;
use openraft::type_config::TypeConfigExt;
use openraft::type_config::alias::OneshotReceiverOf;
use openraft_memstore::ClientRequest;
use openraft_memstore::IntoMemClientRequest;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::rpc_request::RpcRequest;
use crate::fixtures::ut_harness;

/// A removed voter retries a failed automatic campaign against its single remote voter.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn automatic_recovery_retries_single_remote_voter() -> Result<()> {
    for retain in [false, true] {
        for pre_vote in [false, true] {
            tracing::info!(retain, pre_vote, "--- restart with final only on the removed leader");
            let (router, final_index) = restart_in_final_window(retain, pre_vote).await?;
            let removed = router.get_raft_handle(&0)?;
            let survivor = router.get_raft_handle(&1)?;

            tracing::info!(
                final_index,
                "--- fail the first automatic election round to the sole remote voter"
            );
            {
                let failed = block_election_requests(&router).await;
                removed.runtime_config().elect(true);
                TypeConfig::timeout(timeout().unwrap(), failed)
                    .await
                    .context("removed voter did not start an automatic campaign")??;

                let (mut store, _) = router.get_storage_handle(&0)?;
                let saved_vote = store.read_vote().await?.unwrap();
                if pre_vote {
                    assert_eq!(
                        Vote::new_committed(1, 0),
                        saved_vote,
                        "failed PreVote must not advance the term"
                    );
                } else {
                    assert!(
                        saved_vote > Vote::new_committed(1, 0),
                        "a direct campaign advances the term"
                    );
                }
                let state = router.with_raft_state(0, |state| state.server_state).await?;
                assert_ne!(
                    ServerState::Leader,
                    state,
                    "the removed node's self-vote is not a quorum"
                );
            }

            tracing::info!(
                final_index,
                "--- heal voting and require a later automatic round to replicate final"
            );
            {
                router.rpc_pre_hook(RPCTypes::Vote, None).await;
                let received = survivor
                    .wait(timeout())
                    .metrics(
                        |m| m.membership_config.log_id().index() == Some(final_index),
                        "the recovered removed leader replicates final",
                    )
                    .await?;
                assert!(received.current_term > 1, "leader restoration is disabled");
            }

            tracing::info!(
                final_index,
                "--- let the surviving single voter lead, commit final, and serve a write"
            );
            {
                survivor.runtime_config().elect(true);
                survivor.wait(timeout()).state(ServerState::Leader, "surviving voter takes over").await?;
                survivor
                    .wait(timeout())
                    .metrics(
                        |m| {
                            m.committed_membership_config.log_id().index() == Some(final_index)
                                && m.last_applied.index() >= Some(final_index)
                        },
                        "final membership is committed and applied",
                    )
                    .await?;
                survivor.client_write(ClientRequest::make_request("recovered", 1)).await?;
            }

            removed.shutdown().await?;
            survivor.shutdown().await?;
        }
    }
    Ok(())
}

/// Explicit triggers admit a committed voter even when final removes it entirely.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn explicit_recovery_election_reaches_remote_voter() -> Result<()> {
    for pre_vote in [false, true] {
        tracing::info!(
            pre_vote,
            "--- keep automatic elections disabled after the removed leader restarts"
        );
        let (router, final_index) = restart_in_final_window(false, pre_vote).await?;
        let removed = router.get_raft_handle(&0)?;

        tracing::info!(
            final_index,
            "--- the explicit trigger must send a campaign request despite removal"
        );
        {
            let requested = block_election_requests(&router).await;
            removed.trigger().elect(pre_vote).await?;
            TypeConfig::timeout(timeout().unwrap(), requested)
                .await
                .context("explicit election did not reach the remote voter")??;
        }

        removed.shutdown().await?;
        router.get_raft_handle(&1)?.shutdown().await?;
    }
    Ok(())
}

/// Use two public appends to put both nodes at committed joint, then persist final only on node 0.
async fn restart_in_final_window(retain: bool, pre_vote: bool) -> Result<(RaftRouter, u64)> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            enable_elect: false,
            enable_leader_restore: Some(false),
            enable_pre_vote: Some(pre_vote),
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config);
    let mut log_index = router.new_cluster(btreeset! {0, 1}, btreeset! {}).await?;
    let leader = router.get_raft_handle(&0)?;

    tracing::info!(log_index, "--- commit joint on both nodes before blocking replication");
    {
        let joint = Membership::new_with_defaults(vec![btreeset! {0, 1}, btreeset! {1}], []);
        leader.append_membership(joint, EntryPayload::Blank, []).await?;
        log_index += 1;
        for id in [0, 1] {
            router.wait(&id, timeout()).applied_index(Some(log_index), "joint applied on both nodes").await?;
        }
    }

    tracing::info!(
        log_index,
        retain,
        "--- block the original Leader's appends across restart; only a newer Vote may replicate final"
    );
    let pending = {
        router
            .set_rpc_pre_hook(RPCTypes::AppendEntries, |_router, request, from, to| {
                let result = if from == 0
                    && to == 1
                    && matches!(request, RpcRequest::AppendEntries(append) if append.vote == Vote::new_committed(1, 0))
                {
                    Err(RPCError::Network(NetworkError::<TypeConfig>::from_string(
                        "hold final on node 0",
                    )))
                } else {
                    Ok(())
                };
                Box::pin(futures::future::ready(result))
            })
            .await;
        let learners = if retain {
            btreeset! {0}
        } else {
            btreeset! {}
        };
        let final_membership = Membership::new_with_defaults(vec![btreeset! {1}], learners);
        let writer = leader.clone();
        let pending =
            TypeConfig::spawn(async move { writer.append_membership(final_membership, EntryPayload::Blank, []).await });
        log_index += 1;
        let target = Some(FlushPoint::new(
            Vote::new_committed(1, 0),
            Some(log_id(1, 0, log_index)),
        ));
        TypeConfig::timeout(timeout().unwrap(), leader.watch_log_progress().wait_until_ge(&target)).await??;
        pending
    };

    tracing::info!(
        log_index,
        "--- verify the fault window and restart from the unchanged storage"
    );
    {
        let metrics = leader
            .wait(timeout())
            .metrics(
                |m| m.membership_config.log_id().index() == Some(log_index),
                "final is effective before the crash",
            )
            .await?;
        assert_eq!(
            Some(log_index - 1),
            metrics.committed_membership_config.log_id().index()
        );
        assert_eq!(Some(log_index - 1), metrics.local_committed.index());
        let (mut other_store, _) = router.get_storage_handle(&1)?;
        assert_eq!(
            Some(log_index - 1),
            other_store.get_log_state().await?.last_log_id.index(),
            "final has not reached node 1"
        );

        let (node, mut store, sm) = router.remove_node(0).unwrap();
        node.shutdown().await?;
        assert!(TypeConfig::timeout(timeout().unwrap(), pending).await??.is_err());
        assert_eq!(Some(log_id(1, 0, log_index)), store.get_log_state().await?.last_log_id);
        assert_eq!(Some(log_id(1, 0, log_index - 1)), store.read_committed().await?);
        router.new_raft_node_with_sto(0, store, sm).await;
        router
            .wait(&0, timeout())
            .metrics(
                |m| {
                    m.state == ServerState::Learner
                        && m.membership_config.log_id().index() == Some(log_index)
                        && m.committed_membership_config.log_id().index() == Some(log_index - 1)
                },
                "restart retains final without restoring leader authority",
            )
            .await?;
    }

    Ok((router, log_index))
}

/// Both Vote and PreVote use the fixture's Vote hook; a notification proves an actual attempt.
async fn block_election_requests(router: &RaftRouter) -> OneshotReceiverOf<TypeConfig, ()> {
    let (tx, rx) = TypeConfig::oneshot();
    let first_request = Mutex::new(Some(tx));
    router
        .set_rpc_pre_hook(RPCTypes::Vote, move |_router, _request, from, to| {
            let result = if from == 0 && to == 1 {
                if let Some(tx) = first_request.lock().unwrap().take() {
                    let _ = tx.send(());
                }
                Err(RPCError::Network(NetworkError::<TypeConfig>::from_string(
                    "fail recovery election round",
                )))
            } else {
                Ok(())
            };
            Box::pin(futures::future::ready(result))
        })
        .await;
    rx
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_secs(5))
}
