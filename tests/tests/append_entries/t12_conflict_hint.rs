use std::pin::pin;
use std::time::Duration;

use anyhow::Result;
use futures::StreamExt;
use openraft::Config;
use openraft::RaftLogReader;
use openraft::Vote;
use openraft::network::RPCOption;
use openraft::network::RaftNetworkFactory;
use openraft::network::v2::RaftNetworkV2;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::AppendError;
use openraft::raft::ConflictHint;
use openraft::testing::blank_ent;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// Stream conflicts carry a hint that distinguishes an empty log and describes truncation, while
/// the unary API keeps the legacy reply.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn stream_append_reports_conflict_hints() -> Result<()> {
    let config = Config {
        enable_tick: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    router.new_raft_node(0).await?;
    let raft = router.get_raft_handle(&0)?;

    tracing::info!("an empty follower explicitly reports an empty log");
    {
        let request = AppendEntriesRequest {
            vote: Vote::new_committed(1, 1),
            prev_log_id: Some(log_id(1, 1, 5)),
            entries: vec![],
            leader_commit: None,
        };
        let requests = futures::stream::iter([request]);
        let actual: Vec<_> = pin!(raft.stream_append(requests)).collect().await;
        let conflict = AppendError::Conflict {
            log_id: log_id(1, 1, 5),
            hint: Some(ConflictHint::new(vec![])),
        };
        assert_eq!(vec![Ok(Err(conflict))], actual);
    }

    tracing::info!("a successful append acknowledges persisted entries");
    {
        let request = AppendEntriesRequest {
            vote: Vote::new_committed(1, 1),
            prev_log_id: None,
            entries: vec![
                blank_ent::<TypeConfig>(0, 0, 0),
                blank_ent::<TypeConfig>(1, 1, 1),
                blank_ent::<TypeConfig>(1, 1, 2),
            ],
            leader_commit: Some(log_id(0, 0, 0)),
        };
        let requests = futures::stream::iter([request]);
        let actual: Vec<_> = pin!(raft.stream_append(requests)).collect().await;
        assert_eq!(vec![Ok(Ok(Some(log_id(1, 1, 2))))], actual);
    }

    tracing::info!("a newer vote receives the legacy conflict reply from the unary API");
    {
        let request = AppendEntriesRequest {
            vote: Vote::new_committed(2, 1),
            prev_log_id: Some(log_id(2, 1, 5)),
            entries: vec![],
            leader_commit: None,
        };
        let actual = raft.append_entries(request).await?;
        assert_eq!(AppendEntriesResponse::Conflict, actual);
    }

    tracing::info!("after a vote-only flush, the hint still describes the nonempty log after truncation");
    {
        let request = AppendEntriesRequest {
            vote: Vote::new_committed(2, 1),
            prev_log_id: Some(log_id(2, 1, 2)),
            entries: vec![],
            leader_commit: None,
        };
        let requests = futures::stream::iter([request]);
        let actual: Vec<_> = pin!(raft.stream_append(requests)).collect().await;
        let committed = log_id(0, 0, 0);
        let tail = log_id(1, 1, 1);
        let conflict = AppendError::Conflict {
            log_id: log_id(2, 1, 2),
            hint: Some(ConflictHint::new(vec![committed, tail])),
        };
        assert_eq!(vec![Ok(Err(conflict))], actual);
    }
    Ok(())
}

/// Quota truncation preserves the acknowledged prefix on both AppendEntries paths.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn quota_preserves_stream_and_unary_reply_boundaries() -> Result<()> {
    for stream in [false, true] {
        let config = Config {
            enable_tick: false,
            enable_heartbeat: false,
            enable_elect: false,
            ..Default::default()
        };
        let builder = RaftRouter::builder(config)?;
        let mut router = builder.conflict_hints().build();
        router.new_raft_node(0).await?;
        let mut client = RaftNetworkFactory::new_client(&mut router, 0, &()).await;

        for (quota, indices, retained_end) in [
            (Some(0), 0..2, 0),
            (Some(1), 0..2, 1),
            (Some(0), 1..3, 1),
            (Some(2), 1..3, 3),
            (Some(3), 3..5, 5),
            (None, 5..7, 7),
        ] {
            tracing::info!(
                stream,
                ?quota,
                ?indices,
                retained_end,
                "verify the quota-limited reply and stored prefix"
            );
            {
                router.set_append_entries_quota(quota);
                let prev_log_id = if indices.start == 0 {
                    None
                } else {
                    Some(log_id(1, 1, indices.start - 1))
                };
                let end = indices.end;
                let entries = indices.map(|index| blank_ent::<TypeConfig>(1, 1, index)).collect();
                let request = AppendEntriesRequest {
                    vote: Vote::new_committed(1, 1),
                    prev_log_id,
                    entries,
                    leader_commit: None,
                };
                let option = RPCOption::new(Duration::from_secs(1));
                let last_log_id = if retained_end == 0 {
                    None
                } else {
                    Some(log_id(1, 1, retained_end - 1))
                };
                if stream {
                    let requests = futures::stream::iter([request]);
                    let responses = client.stream_append(requests, option).await?;
                    let actual: Vec<_> = responses.collect().await;
                    assert_eq!(vec![Ok(Ok(last_log_id))], actual);
                } else {
                    let actual = client.append_entries(request, option).await?;
                    let expected = if retained_end < end {
                        AppendEntriesResponse::PartialSuccess(last_log_id)
                    } else {
                        AppendEntriesResponse::Success
                    };
                    assert_eq!(expected, actual);
                }

                let (mut store, _) = router.get_storage_handle(&0)?;
                let actual_entries = store.try_get_log_entries(..).await?;
                let actual = bincode::serialize(&actual_entries)?;
                let expected_entries: Vec<_> =
                    (0..retained_end).map(|index| blank_ent::<TypeConfig>(1, 1, index)).collect();
                let expected = bincode::serialize(&expected_entries)?;
                assert_eq!(expected, actual);
            }
        }
    }
    Ok(())
}
