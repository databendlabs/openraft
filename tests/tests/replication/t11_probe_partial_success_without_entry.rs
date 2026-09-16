use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::RPCTypes;
use openraft::ServerState;
use openraft::async_runtime::MpscReceiver;
use openraft::async_runtime::MpscSender;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::rpc_request::RpcRequest;
use crate::fixtures::ut_harness;

/// A matching-point probe answered with `PartialSuccess(prev_log_id)` must be sent again.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn probe_partial_success_without_entry_is_retried() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_elect: false,
            // Heartbeats refresh the leader lease, which would reject the election below.
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    tracing::info!(log_index, "--- write logs so a probe has a search range to bisect");
    {
        log_index += router.client_request_many(0, "foo", 10).await?;

        for id in [0, 1, 2] {
            router.wait(&id, timeout()).applied_index(Some(log_index), "10 writes").await?;
        }
    }

    let (retried_tx, mut retried_rx) = TypeConfig::mpsc(2);

    tracing::info!(
        log_index,
        "--- observe the second entry-carrying request to each follower"
    );
    {
        let attempts = [AtomicU64::new(0), AtomicU64::new(0)];

        router
            .set_rpc_pre_hook(RPCTypes::AppendEntries, move |_router, req, from, to| {
                let mut retried = false;
                if from == 1
                    && let RpcRequest::AppendEntries(append) = &req
                    && !append.entries.is_empty()
                {
                    let index = match to {
                        0 => 0,
                        2 => 1,
                        _ => unreachable!("unexpected follower {to}"),
                    };
                    retried = attempts[index].fetch_add(1, Ordering::Relaxed) == 1;
                }
                let retried_tx = retried_tx.clone();
                Box::pin(async move {
                    if retried {
                        retried_tx.send(to).await.unwrap();
                    }
                    Ok(())
                })
            })
            .await;
    }

    tracing::info!(log_index, "--- make every entry-carrying AppendEntries accept nothing");
    {
        router.set_append_entries_quota(Some(0));

        // The lease the old leader holds on nodes 0 and 2 has to expire before node 1 can win.
        TypeConfig::sleep(Duration::from_millis(1_000)).await;
    }

    tracing::info!(log_index, "--- elect node 1: its progress restarts in probing mode");
    {
        router.get_raft_handle(&1)?.trigger().elect(false).await?;
        router.wait(&1, timeout()).state(ServerState::Leader, "node 1 is leader").await?;

        // The blank log node 1 appends at the start of its term.
        log_index += 1;
    }

    tracing::info!(
        log_index,
        "--- both followers must retry after acknowledging no entry beyond prev"
    );
    {
        let retried = TypeConfig::timeout(Duration::from_secs(5), async {
            let first = retried_rx.recv().await.expect("retry hook closed");
            let second = retried_rx.recv().await.expect("retry hook closed");
            btreeset! {first, second}
        })
        .await?;
        assert_eq!(btreeset! {0, 2}, retried);
    }

    tracing::info!(
        log_index,
        "--- lift the quota: the retried probes must catch the followers up"
    );
    {
        router.set_append_entries_quota(None);

        router.wait(&0, timeout()).applied_index(Some(log_index), "node 0 caught up").await?;
        router.wait(&2, timeout()).applied_index(Some(log_index), "node 2 caught up").await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(5_000))
}
