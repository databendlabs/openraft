use std::pin::pin;
use std::sync::Arc;

use anyhow::Result;
use futures::StreamExt;
use openraft::Config;
use openraft::Vote;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::StreamAppendError;
use openraft::raft::VoteRequest;
use openraft::testing::blank_ent;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// Test stream_append with successful append entries.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn stream_append_success() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    router.new_raft_node(0).await;

    let raft = router.get_raft_handle(&0)?;

    let requests = vec![
        AppendEntriesRequest::<openraft_memstore::TypeConfig> {
            vote: Vote::new_committed(1, 1),
            prev_log_id: None,
            entries: vec![blank_ent(0, 0, 0), blank_ent(1, 1, 1)],
            leader_commit: None,
        },
        AppendEntriesRequest::<openraft_memstore::TypeConfig> {
            vote: Vote::new_committed(1, 1),
            prev_log_id: Some(log_id(1, 1, 1)),
            entries: vec![blank_ent(1, 1, 2), blank_ent(1, 1, 3)],
            leader_commit: None,
        },
        AppendEntriesRequest::<openraft_memstore::TypeConfig> {
            vote: Vote::new_committed(1, 1),
            prev_log_id: Some(log_id(1, 1, 3)),
            entries: vec![blank_ent(1, 1, 4)],
            leader_commit: Some(log_id(1, 1, 4)),
        },
    ];

    let input_stream = futures::stream::iter(requests);
    let output_stream = pin!(raft.stream_append(input_stream));

    let results: Vec<_> = output_stream.collect().await;
    assert_eq!(results, vec![
        Ok(Some(log_id(1, 1, 1))),
        Ok(Some(log_id(1, 1, 3))),
        Ok(Some(log_id(1, 1, 4))),
    ]);

    Ok(())
}

/// Test stream_append terminates on conflict.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn stream_append_conflict() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    router.new_raft_node(0).await;

    let raft = router.get_raft_handle(&0)?;

    // First request succeeds, second has conflicting prev_log_id
    let requests = vec![
        AppendEntriesRequest::<openraft_memstore::TypeConfig> {
            vote: Vote::new_committed(1, 1),
            prev_log_id: None,
            entries: vec![blank_ent(0, 0, 0), blank_ent(1, 1, 1)],
            leader_commit: None,
        },
        // This will conflict: prev_log_id at index 5 doesn't exist
        AppendEntriesRequest::<openraft_memstore::TypeConfig> {
            vote: Vote::new_committed(1, 1),
            prev_log_id: Some(log_id(1, 1, 5)),
            entries: vec![blank_ent(1, 1, 6)],
            leader_commit: None,
        },
        // This should never be processed because stream terminates on conflict
        AppendEntriesRequest::<openraft_memstore::TypeConfig> {
            vote: Vote::new_committed(1, 1),
            prev_log_id: Some(log_id(1, 1, 6)),
            entries: vec![blank_ent(1, 1, 7)],
            leader_commit: None,
        },
    ];

    let input_stream = futures::stream::iter(requests);
    let output_stream = pin!(raft.stream_append(input_stream));

    let results: Vec<_> = output_stream.collect().await;
    assert_eq!(results, vec![
        Ok(Some(log_id(1, 1, 1))),
        Err(StreamAppendError::Conflict(Some(log_id(1, 1, 5)))),
    ]);

    Ok(())
}

/// Test stream_append terminates on higher vote.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn stream_append_higher_vote() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    router.new_raft_node(0).await;

    let raft = router.get_raft_handle(&0)?;

    // Establish a higher vote on the node
    let resp = raft
        .vote(VoteRequest {
            vote: Vote::new(10, 2),
            last_log_id: Some(log_id(10, 2, 100)),
        })
        .await?;
    assert!(resp.is_granted_to(&Vote::new(10, 2)));

    // Now try to append with a lower vote
    let requests = vec![
        AppendEntriesRequest::<openraft_memstore::TypeConfig> {
            vote: Vote::new_committed(1, 1), // Lower than (10, 2)
            prev_log_id: None,
            entries: vec![blank_ent(0, 0, 0)],
            leader_commit: None,
        },
        // This should never be processed
        AppendEntriesRequest::<openraft_memstore::TypeConfig> {
            vote: Vote::new_committed(1, 1),
            prev_log_id: Some(log_id(0, 0, 0)),
            entries: vec![blank_ent(1, 1, 1)],
            leader_commit: None,
        },
    ];

    let input_stream = futures::stream::iter(requests);
    let output_stream = pin!(raft.stream_append(input_stream));

    let results: Vec<_> = output_stream.collect().await;
    assert_eq!(results, vec![Err(StreamAppendError::HigherVote(Vote::new(10, 2))),]);

    Ok(())
}
