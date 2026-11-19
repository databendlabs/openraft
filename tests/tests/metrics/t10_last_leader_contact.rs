use anyhow::Result;
use openraft::Config;
use openraft::Vote;
use openraft::async_runtime::WatchReceiver;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendError;
use openraft::raft::VoteRequest;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// A restored vote is not contact, and only replication for the current leader counts.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn last_leader_contact() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    router.new_raft_node(0).await?;

    let leader_vote = Vote::new_committed(2, 1);
    tracing::info!(%leader_vote, "--- record a heartbeat, then restart with the vote still on disk");
    {
        let node = router.get_raft_handle(&0)?;
        let result = node
            .append_entries(AppendEntriesRequest {
                vote: leader_vote,
                prev_log_id: None,
                entries: vec![],
                leader_commit: None,
            })
            .await?;
        assert_eq!(result, Ok(None));
        router.wait(&0).metrics(|m| m.last_leader_contact.is_some(), "heartbeat recorded").await?;

        let stopped = router.stop_node(0).await?;
        router.new_raft_node_with_sto(0, stopped.log_store, stopped.state_machine).await?;
        let metrics = router.wait(&0).current_leader(1, "restored leader vote").await?;
        assert_eq!(metrics.last_leader_contact, None);
    }

    let node = router.get_raft_handle(&0)?;
    tracing::info!(%leader_vote, "--- an append with a log conflict still proves contact");
    let contact = {
        let result = node
            .append_entries(AppendEntriesRequest {
                vote: leader_vote,
                prev_log_id: Some(log_id(2, 1, 1)),
                entries: vec![],
                leader_commit: None,
            })
            .await?;
        assert!(matches!(result, Err(AppendError::Conflict { .. })));
        let metrics =
            router.wait(&0).metrics(|m| m.last_leader_contact.is_some(), "conflicting append recorded").await?;
        assert_eq!(
            node.data_metrics().borrow_watched().last_leader_contact,
            metrics.last_leader_contact
        );
        let contact = metrics.last_leader_contact.unwrap().into_inner();
        assert_eq!(node.with_raft_state(|s| s.last_leader_contact()).await?, Some(contact));
        contact
    };

    tracing::info!(%leader_vote, "--- a rejected older vote must not refresh contact");
    {
        let result = node
            .append_entries(AppendEntriesRequest {
                vote: Vote::new_committed(1, 1),
                prev_log_id: None,
                entries: vec![],
                leader_commit: None,
            })
            .await?;
        assert_eq!(result, Err(AppendError::HigherVote(leader_vote)));
        assert_eq!(node.with_raft_state(|s| s.last_leader_contact()).await?, Some(contact));
    }

    let next_vote = Vote::new(3, 1);
    tracing::info!(%next_vote, "--- a new campaign from the same peer clears the previous leader's contact");
    {
        let response = node.vote(VoteRequest::new(next_vote, None)).await?;
        assert!(response.vote_granted);
        let metrics = router.wait(&0).vote(next_vote, "new vote").await?;
        assert_eq!(metrics.last_leader_contact, None);
    }

    Ok(())
}
