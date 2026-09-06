use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;
use maplit::btreemap;
use maplit::btreeset;
use openraft::Config;
use openraft::RPCTypes;
use openraft::ServerState;
use openraft::async_runtime::MpscReceiver;
use openraft::async_runtime::MpscSender;
use openraft::type_config::TypeConfigExt;
use openraft::type_config::alias::LogIdOf;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::rpc_request::RpcRequest;
use crate::fixtures::ut_harness;

/// The `prev_log_id` of the first two entry-carrying AppendEntries sent to each follower.
type FirstTwoProbes = BTreeMap<u64, Vec<Option<LogIdOf<TypeConfig>>>>;

/// A probe at the empty log position answered with `PartialSuccess(None)` is re-issued: the
/// acknowledgement reaches the engine and clears the inflight probe instead of leaving it stuck.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn probe_acked_at_empty_prev_is_reissued() -> Result<()> {
    // With 8 writes the leader's log ends at index 14, so the search span is under 16 and
    // `calc_mid` keeps every probe start at `None`.
    let probes = first_two_probes_with_quota_zero(8).await?;

    let expected: FirstTwoProbes = btreemap! {
        0 => vec![None, None],
        2 => vec![None, None],
    };
    assert_eq!(expected, probes);

    Ok(())
}

/// A probe answered with `PartialSuccess(prev)` completes, and the next probe starts at a later
/// midpoint instead of re-sending the same range.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn probe_acked_at_prev_advances_to_next_midpoint() -> Result<()> {
    // With 40 writes the leader's log ends at index 46. `calc_mid` moves the probe start by
    // `span / 16 * 8`: the first probe starts after index 15, and once `matching` is 15 the
    // second starts after index 23.
    let probes = first_two_probes_with_quota_zero(40).await?;

    let expected: FirstTwoProbes = btreemap! {
        0 => vec![Some(log_id(1, 0, 15)), Some(log_id(1, 0, 23))],
        2 => vec![Some(log_id(1, 0, 15)), Some(log_id(1, 0, 23))],
    };
    assert_eq!(expected, probes);

    Ok(())
}

/// Write `n_writes` logs, then elect node 1 while every entry-carrying AppendEntries is answered
/// with `PartialSuccess(prev_log_id)`, so node 1 searches for the matching point with probes that
/// carry entries but never get one accepted.
///
/// Returns the `prev_log_id` of the first two such probes node 1 sends to each follower, after
/// checking that lifting the quota lets the probes catch the followers up.
async fn first_two_probes_with_quota_zero(n_writes: usize) -> Result<FirstTwoProbes> {
    let config = Arc::new(
        Config {
            enable_elect: false,
            // Heartbeats refresh the leader lease, which would reject the election below.
            enable_heartbeat: false,
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    tracing::info!(
        log_index,
        n_writes,
        "--- write logs so a probe has a search range to bisect"
    );
    {
        log_index += router.client_request_many(0, "foo", n_writes).await?;

        for id in [0, 1, 2] {
            router.wait(&id, timeout()).applied_index(Some(log_index), format!("{n_writes} writes")).await?;
        }
    }

    let probes = Arc::new(Mutex::new(FirstTwoProbes::new()));
    let (second_tx, mut second_rx) = TypeConfig::mpsc(2);

    tracing::info!(
        log_index,
        "--- record the first two entry-carrying requests to each follower"
    );
    {
        let recorder = probes.clone();

        router
            .set_rpc_pre_hook(RPCTypes::AppendEntries, move |_router, req, from, to| {
                let mut second = false;
                if from == 1
                    && let RpcRequest::AppendEntries(append) = &req
                    && !append.entries.is_empty()
                {
                    let mut recorded = recorder.lock().unwrap();
                    let prevs = recorded.entry(to).or_default();
                    if prevs.len() < 2 {
                        prevs.push(append.prev_log_id);
                        second = prevs.len() == 2;
                    }
                }
                let second_tx = second_tx.clone();
                Box::pin(async move {
                    if second {
                        second_tx.send(to).await.unwrap();
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

    tracing::info!(log_index, "--- both followers receive a second entry-carrying request");
    {
        let continued = TypeConfig::timeout(Duration::from_secs(5), async {
            let first = second_rx.recv().await.expect("probe hook closed");
            let second = second_rx.recv().await.expect("probe hook closed");
            btreeset! {first, second}
        })
        .await?;
        assert_eq!(btreeset! {0, 2}, continued);
    }

    tracing::info!(log_index, "--- lift the quota: the probes must catch the followers up");
    {
        router.set_append_entries_quota(None);

        router.wait(&0, timeout()).applied_index(Some(log_index), "node 0 caught up").await?;
        router.wait(&2, timeout()).applied_index(Some(log_index), "node 2 caught up").await?;
    }

    let probes = probes.lock().unwrap();
    Ok(probes.clone())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(5_000))
}
