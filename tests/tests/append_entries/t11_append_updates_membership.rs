use std::option::Option::None;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::raft::AppendEntriesRequest;
use openraft::testing::blank_ent;
use openraft::CommittedLeaderId;
use openraft::Config;
use openraft::Entry;
use openraft::EntryPayload;
use openraft::LogId;
use openraft::Membership;
use openraft::ServerState;
use openraft::Vote;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// append-entries should update membership correctly when adding new logs and deleting
/// inconsistent logs.
///
/// - bring up a learner and send to it append_entries request. Check the membership updated.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn append_updates_membership() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    router.new_raft_node(0).await;

    tracing::info!("--- wait for init node to ready");

    router.wait_for_log(&btreeset![0], None, None, "empty").await?;
    router.wait_for_state(&btreeset![0], ServerState::Learner, None, "empty").await?;

    let (r0, _sto0, _sm0) = router.remove_node(0).unwrap();

    tracing::info!("--- append-entries update membership");
    {
        let req = AppendEntriesRequest {
            vote: Vote::new_committed(1, 1),
            prev_log_id: None,
            entries: vec![
                blank_ent(0, 0, 0),
                blank_ent(1, 0, 1),
                Entry {
                    log_id: LogId::new(CommittedLeaderId::new(1, 0), 2),
                    payload: EntryPayload::Membership(Membership::new(vec![btreeset! {1,2}], None)),
                },
                blank_ent(1, 0, 3),
                Entry {
                    log_id: LogId::new(CommittedLeaderId::new(1, 0), 4),
                    payload: EntryPayload::Membership(Membership::new(vec![btreeset! {1,2,3,4}], None)),
                },
                blank_ent(1, 0, 5),
            ],
            leader_commit: Some(LogId::new(CommittedLeaderId::new(0, 0), 0)),
        };

        let resp = r0.append_entries(req).await?;
        assert!(resp.is_success());
        assert!(!resp.is_conflict());

        r0.wait(timeout()).voter_ids([1, 2, 3, 4], "append-entries update membership").await?;
    }

    tracing::info!("--- delete inconsistent logs update membership");
    {
        let req = AppendEntriesRequest {
            vote: Vote::new_committed(2, 2),
            prev_log_id: Some(LogId::new(CommittedLeaderId::new(1, 0), 2)),
            entries: vec![blank_ent(2, 0, 3)],
            leader_commit: Some(LogId::new(CommittedLeaderId::new(0, 0), 0)),
        };

        let resp = r0.append_entries(req).await?;
        assert!(resp.is_success());
        assert!(!resp.is_conflict());

        r0.wait(timeout()).voter_ids([1, 2], "deleting inconsistent logs updates membership").await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(5000))
}
