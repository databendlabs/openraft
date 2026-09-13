use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::RPCTypes;
use openraft::async_runtime::WatchReceiver;
use openraft::errors::RPCError;
use openraft::errors::Unreachable;
use openraft::type_config::OneshotSender;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::rpc_request::RpcRequest;
use crate::fixtures::rpc_response::RpcResponse;
use crate::fixtures::ut_harness;

/// A PartialSuccess response proves that the follower accepted the Leader's vote, so it must
/// refresh that follower's clock progress even though it did not accept the complete request.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn partial_success_updates_clock_progress() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config);

    tracing::info!("--- initialize a 3-voter cluster");
    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    tracing::info!(log_index, "--- isolate voter 1 and append one log through voter 2");
    {
        router.set_unreachable(1, true);
        log_index += router.client_request_many(0, "partial", 1).await?;
    }

    let (partial_tx, partial_rx) = TypeConfig::oneshot();
    let partial_tx = Arc::new(Mutex::new(Some(partial_tx)));
    let request_had_entries = Arc::new(AtomicBool::new(false));

    tracing::info!(log_index, "--- accept no entries, then isolate voter 1 again");
    {
        router.set_append_entries_quota(Some(0));

        let request_had_entries_pre = request_had_entries.clone();
        router
            .set_rpc_pre_hook(RPCTypes::AppendEntries, move |_router, req, _from, target| {
                let result = if target == 1 {
                    let non_empty = matches!(&req, RpcRequest::AppendEntries(req) if !req.entries.is_empty());
                    request_had_entries_pre.store(non_empty, Ordering::SeqCst);

                    if non_empty {
                        Ok(())
                    } else {
                        Err(RPCError::Unreachable(Unreachable::<TypeConfig>::from_string(
                            "skip commit-only request",
                        )))
                    }
                } else {
                    Ok(())
                };

                Box::pin(futures::future::ready(result))
            })
            .await;

        router
            .set_rpc_post_hook(RPCTypes::AppendEntries, move |router, req, resp, _from, target| {
                if target == 1 {
                    let is_strict_partial = request_had_entries.swap(false, Ordering::SeqCst)
                        && matches!(&req, RpcRequest::AppendEntries(req) if req.entries.is_empty())
                        && matches!(&resp, RpcResponse::AppendEntries(resp) if resp.is_success());
                    assert!(
                        is_strict_partial,
                        "unexpected response before PartialSuccess: req={req:?}, resp={resp:?}"
                    );

                    router.set_unreachable(target, true);

                    if let Some(tx) = partial_tx.lock().unwrap().take() {
                        let clock_before_response = router
                            .get_raft_handle(&0)
                            .unwrap()
                            .metrics()
                            .borrow_watched()
                            .heartbeat
                            .as_ref()
                            .and_then(|clock| clock.get(&1))
                            .copied()
                            .flatten();
                        tx.send(clock_before_response).ok();
                    }
                }

                Box::pin(async { Ok(()) })
            })
            .await;
    }

    tracing::info!(log_index, "--- reconnect voter 1 and receive one PartialSuccess");
    let clock_before_response = {
        router.set_unreachable(1, false);
        TypeConfig::timeout(Duration::from_secs(2), partial_rx).await??.map(|acked| acked.into_inner())
    };

    tracing::info!(log_index, "--- PartialSuccess refreshes voter 1 clock progress");
    {
        let n0 = router.get_raft_handle(&0)?;
        n0.wait(Some(Duration::from_secs(1)))
            .metrics(
                |metrics| {
                    metrics
                        .heartbeat
                        .as_ref()
                        .and_then(|clock| clock.get(&1))
                        .copied()
                        .flatten()
                        .is_some_and(|acked| clock_before_response.is_none_or(|before| acked.into_inner() > before))
                },
                "PartialSuccess refreshes clock progress",
            )
            .await?;
    }

    Ok(())
}
