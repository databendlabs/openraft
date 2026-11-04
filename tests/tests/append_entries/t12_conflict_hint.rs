use std::time::Duration;

use anyhow::Result;
use openraft::Config;
use openraft::RaftLogReader;
use openraft::Vote;
use openraft::network::RPCOption;
use openraft::network::RaftNetworkFactory;
use openraft::network::v2::RaftNetworkV2;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendError;
use openraft::raft::ConflictHint;
use openraft::testing::blank_ent;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// Conflict replies carry a hint that distinguishes an empty log and describes the log after
/// truncation.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn append_entries_reports_conflict_hints() -> Result<()> {
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
        let actual = raft.append_entries(request).await?;
        let conflict = AppendError::Conflict {
            log_id: log_id(1, 1, 5),
            hint: Some(ConflictHint::new(vec![])),
        };
        assert_eq!(Err(conflict), actual);
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
        let actual = raft.append_entries(request).await?;
        assert_eq!(Ok(Some(log_id(1, 1, 2))), actual);
    }

    tracing::info!("a newer vote's conflict past the tail keeps the whole log in the hint");
    {
        let request = AppendEntriesRequest {
            vote: Vote::new_committed(2, 1),
            prev_log_id: Some(log_id(2, 1, 5)),
            entries: vec![],
            leader_commit: None,
        };
        let actual = raft.append_entries(request).await?;
        let committed = log_id(0, 0, 0);
        let tail = log_id(1, 1, 2);
        let conflict = AppendError::Conflict {
            log_id: log_id(2, 1, 5),
            hint: Some(ConflictHint::new(vec![committed, tail])),
        };
        assert_eq!(Err(conflict), actual);
    }

    tracing::info!("after a vote-only flush, the hint still describes the nonempty log after truncation");
    {
        let request = AppendEntriesRequest {
            vote: Vote::new_committed(2, 1),
            prev_log_id: Some(log_id(2, 1, 2)),
            entries: vec![],
            leader_commit: None,
        };
        let actual = raft.append_entries(request).await?;
        let committed = log_id(0, 0, 0);
        let tail = log_id(1, 1, 1);
        let conflict = AppendError::Conflict {
            log_id: log_id(2, 1, 2),
            hint: Some(ConflictHint::new(vec![committed, tail])),
        };
        assert_eq!(Err(conflict), actual);
    }
    Ok(())
}

/// Quota truncation preserves the acknowledged prefix with and without hints: the legacy reply
/// format is rebuilt from the request that was sent.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn quota_preserves_hinted_and_legacy_reply_boundaries() -> Result<()> {
    for hints in [false, true] {
        let config = Config {
            enable_tick: false,
            enable_heartbeat: false,
            enable_elect: false,
            ..Default::default()
        };
        let builder = RaftRouter::builder(config)?;
        let builder = if hints { builder.conflict_hints() } else { builder };
        let mut router = builder.build();
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
                hints,
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
                let entries = indices.map(|index| blank_ent::<TypeConfig>(1, 1, index)).collect();
                let request = AppendEntriesRequest {
                    vote: Vote::new_committed(1, 1),
                    prev_log_id,
                    entries,
                    leader_commit: None,
                };
                let option = RPCOption::new(Duration::from_secs(1));
                let actual = client.append_entries(request, option).await?;
                let last_log_id = if retained_end == 0 {
                    None
                } else {
                    Some(log_id(1, 1, retained_end - 1))
                };
                assert_eq!(Ok(last_log_id), actual);

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
