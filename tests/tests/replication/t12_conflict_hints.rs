use std::sync::Arc;
use std::sync::Mutex;

use anyhow::Result;
use openraft::Config;
use openraft::RaftLogReader;
use openraft::Vote;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::rpc_kind::RpcKind;
use crate::fixtures::rpc_request::RpcRequest;
use crate::fixtures::ut_harness;

const WRITES: usize = 64;
const PREFIX_END: u64 = 20;

/// A lagging follower's hint skips directly to its tail; legacy transports retain binary search.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn conflict_hints_skip_lagging_prefix_search() -> Result<()> {
    for hints in [false, true] {
        let config = Config {
            enable_tick: false,
            enable_heartbeat: false,
            ..Default::default()
        };
        let builder = RaftRouter::builder(config)?;
        let builder = if hints { builder.conflict_hints() } else { builder };
        let mut router = builder.build();
        let mut log_index = router.new_cluster([0], []).await?;

        tracing::info!(hints, log_index, "prepare a leader with a longer committed log");
        {
            router.client_request_many(0, "hints", WRITES).await?;
            log_index += WRITES as u64;
            router.wait_applied_at_least([0], log_index, "leader log ready").await?;
        }

        tracing::info!(hints, "seed follower 1 with the leader's exact prefix through index 20");
        {
            router.new_raft_node(1).await?;
            let (mut store, _) = router.get_storage_handle(&0)?;
            let entries = store.try_get_log_entries(..=PREFIX_END).await?;
            let request = AppendEntriesRequest {
                vote: Vote::new_committed(1, 0),
                prev_log_id: None,
                entries,
                leader_commit: Some(log_id(1, 0, PREFIX_END)),
            };
            let follower = router.get_raft_handle(&1)?;
            let actual = follower.append_entries(request).await?;
            assert_eq!(AppendEntriesResponse::Success, actual);
        }

        let probes = Arc::new(Mutex::new(Vec::new()));
        let recorded = probes.clone();
        router
            .set_rpc_pre_hook(RpcKind::AppendEntries, move |call| {
                if let RpcRequest::AppendEntries(request) = call.request {
                    let to_follower = call.to == 1;
                    let has_entries = !request.entries.is_empty();
                    if to_follower && has_entries {
                        recorded.lock().unwrap().push(request.prev_log_id);
                    }
                }
                futures::future::ready(Ok(()))
            })
            .await;

        tracing::info!(hints, log_index, "add the learner and wait for full application");
        {
            let leader = router.get_raft_handle(&0)?;
            leader.add_learner(1, (), true).await?;
            log_index += 1;
            router.wait_applied_at_least([1], log_index, "learner caught up").await?;
        }

        let mut expected = vec![Some(log_id(0, 0, 0)), Some(log_id(1, 0, 32))];
        if hints {
            expected.extend([Some(log_id(1, 0, PREFIX_END)), Some(log_id(1, 0, 28))]);
        } else {
            expected.extend([Some(log_id(1, 0, 16)), Some(log_id(1, 0, 24)), Some(log_id(1, 0, 32))]);
        }
        let actual = probes.lock().unwrap().clone();
        assert_eq!(expected, actual);

        tracing::info!(
            hints,
            log_index,
            "verify that both transports reproduce the entire leader log"
        );
        {
            let (mut leader, _) = router.get_storage_handle(&0)?;
            let expected_entries = leader.try_get_log_entries(..).await?;
            let expected = bincode::serialize(&expected_entries)?;
            let (mut follower, _) = router.get_storage_handle(&1)?;
            let actual_entries = follower.try_get_log_entries(..).await?;
            let actual = bincode::serialize(&actual_entries)?;
            assert_eq!(expected, actual);
        }
    }
    Ok(())
}
