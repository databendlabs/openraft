use std::collections::HashMap;
use std::time::Duration;

use anyhow::Result;
use openraft::Config;
use openraft::impls::Entry;
use openraft::impls::EntryPayload;
use openraft::impls::Vote;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::testing::blank_ent;
use openraft_memstore::ClientRequest;
use openraft_memstore::IntoMemClientRequest;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::expect_timeout;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// Replay issue #2245's stale request against a follower without running the sender's race.
/// The overwritten entry must stay unapplied until logs from the newer leader replace it.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn stale_vote_with_newer_commit_does_not_apply_overwritten_entry() -> Result<()> {
    const FOLLOWER: u64 = 4;
    let config = Config {
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    router.new_raft_node(FOLLOWER).await?;
    let follower = router.get_raft_handle(&FOLLOWER)?;
    let (_, sm) = router.get_storage_handle(&FOLLOWER)?;
    let prefix_end = log_id(1, 0, 8);
    let stale_log_id = log_id(1, 0, 9);
    let later_commit = log_id(3, 0, 13);

    tracing::info!(%prefix_end, "--- establish the committed prefix before the overwritten entry");
    {
        let entries = (0..=prefix_end.index).map(|index| blank_ent::<TypeConfig>(1, 0, index));
        let request = AppendEntriesRequest {
            vote: Vote::new_committed(1, 0),
            prev_log_id: None,
            entries: entries.collect(),
            leader_commit: Some(prefix_end),
        };
        let response = follower.append_entries(request).await?;
        assert_eq!(AppendEntriesResponse::Success, response);
        router.wait(&FOLLOWER).applied_index(Some(prefix_end.index), "prefix applied").await?;
    }

    tracing::info!(%stale_log_id, %later_commit, "--- replay the stale stream's old vote and entry with the later commit");
    {
        let data = ClientRequest::make_request("x", 1);
        let entry = Entry {
            log_id: stale_log_id,
            payload: EntryPayload::Normal(data),
        };
        let request = AppendEntriesRequest {
            vote: Vote::new_committed(1, 0),
            prev_log_id: Some(prefix_end),
            entries: vec![entry],
            leader_commit: Some(later_commit),
        };
        let response = follower.append_entries(request).await?;
        assert_eq!(AppendEntriesResponse::Success, response);
        router.wait(&FOLLOWER).log_index(Some(stale_log_id.index), "stale entry stored").await?;

        let applied = router
            .wait_with_timeout(&FOLLOWER, Duration::from_millis(200))
            .applied_index_at_least(Some(stale_log_id.index), "stale entry must stay unapplied")
            .await;
        let state = sm.get_state_machine().await;
        assert_eq!(Some(prefix_end), state.last_applied_log);
        let expected_clients = HashMap::new();
        assert_eq!(expected_clients, state.client_status);
        expect_timeout(applied)?;
    }

    tracing::info!(%later_commit, "--- the current leader replaces X and commits the canonical log");
    {
        let data = ClientRequest::make_request("y", 1);
        let replacement = Entry {
            log_id: log_id(2, 1, stale_log_id.index),
            payload: EntryPayload::Normal(data),
        };
        let mut entries = vec![replacement];
        for index in stale_log_id.index + 1..=later_commit.index {
            entries.push(blank_ent::<TypeConfig>(3, 0, index));
        }
        let request = AppendEntriesRequest {
            vote: Vote::new_committed(3, 0),
            prev_log_id: Some(prefix_end),
            entries,
            leader_commit: Some(later_commit),
        };
        let response = follower.append_entries(request).await?;
        assert_eq!(AppendEntriesResponse::Success, response);
        router.wait(&FOLLOWER).applied_index(Some(later_commit.index), "canonical log applied").await?;
        let state = sm.get_state_machine().await;
        assert_eq!(Some(later_commit), state.last_applied_log);
        let expected_clients = HashMap::from([("y".to_string(), "request-1".to_string())]);
        assert_eq!(expected_clients, state.client_status);
    }

    Ok(())
}
