use anyhow::Result;
use openraft::Config;
use openraft::Vote;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::AppendEntriesResponseWithHint;
use openraft::raft::ConflictHint;
use openraft::testing::blank_ent;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// Optional replies distinguish empty logs, describe truncation, and preserve legacy replies.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn append_entries_with_hint() -> Result<()> {
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
        let actual = raft.append_entries_with_hint(request).await?;
        let hint = ConflictHint::new(vec![]);
        let expected = AppendEntriesResponseWithHint::new(AppendEntriesResponse::Conflict, Some(hint));
        assert_eq!(expected, actual);
    }

    tracing::info!("a successful probe acknowledges persisted entries without a conflict hint");
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
        let actual = raft.append_entries_with_hint(request).await?;
        let expected = AppendEntriesResponseWithHint::new(AppendEntriesResponse::Success, None);
        assert_eq!(expected, actual);
    }

    tracing::info!("a newer vote retains the legacy conflict response");
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

    tracing::info!("after a vote-only flush, hints still describe the nonempty log after truncation");
    {
        let request = AppendEntriesRequest {
            vote: Vote::new_committed(2, 1),
            prev_log_id: Some(log_id(2, 1, 2)),
            entries: vec![],
            leader_commit: None,
        };
        let actual = raft.append_entries_with_hint(request).await?;
        let committed = log_id(0, 0, 0);
        let tail = log_id(1, 1, 1);
        let hint = ConflictHint::new(vec![committed, tail]);
        let expected = AppendEntriesResponseWithHint::new(AppendEntriesResponse::Conflict, Some(hint));
        assert_eq!(expected, actual);
    }
    Ok(())
}
