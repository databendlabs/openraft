use std::option::Option::None;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::raft::AppendEntriesRequest;
use openraft::Config;
use openraft::Entry;
use openraft::EntryPayload;
use openraft::LeaderId;
use openraft::LogId;
use openraft::Membership;
use openraft::State;
use openraft::Vote;

use crate::fixtures::blank;
use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// append-entries should update membership correctly when adding new logs and deleting
/// inconsistent logs.
///
/// - bring up a learner and send to it append_entries request. Check the membership updated.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn append_updates_membership() -> Result<()> {
    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let mut router = RaftRouter::new(config.clone());
    router.new_raft_node(0).await;

    tracing::info!("--- wait for init node to ready");

    router.wait_for_log(&btreeset![0], None, None, "empty").await?;
    router.wait_for_state(&btreeset![0], State::Learner, None, "empty").await?;

    let (r0, _sto0) = router.remove_node(0).await.unwrap();

    tracing::info!("--- append-entries update membership");
    {
        let req = AppendEntriesRequest {
            vote: Vote::new(1, 0),
            prev_log_id: None,
            entries: vec![
                blank(0, 0),
                blank(1, 1),
                Entry {
                    log_id: LogId::new(LeaderId::new(1, 0), 2),
                    payload: EntryPayload::Membership(Membership::new(vec![btreeset! {1,2}], None)),
                },
                blank(1, 3),
                Entry {
                    log_id: LogId::new(LeaderId::new(1, 0), 4),
                    payload: EntryPayload::Membership(Membership::new(vec![btreeset! {1,2,3,4}], None)),
                },
                blank(1, 5),
            ],
            leader_commit: Some(LogId::new(LeaderId::new(0, 0), 0)),
        };

        let resp = r0.append_entries(req.clone()).await?;
        assert!(resp.is_success());
        assert!(!resp.is_conflict());

        r0.wait(timeout()).members(btreeset! {1,2,3,4}, "append-entries update membership").await?;
    }

    tracing::info!("--- delete inconsistent logs update membership");
    {
        let req = AppendEntriesRequest {
            vote: Vote::new(1, 0),
            prev_log_id: Some(LogId::new(LeaderId::new(1, 0), 2)),
            entries: vec![blank(2, 3)],
            leader_commit: Some(LogId::new(LeaderId::new(0, 0), 0)),
        };

        let resp = r0.append_entries(req.clone()).await?;
        assert!(resp.is_success());
        assert!(!resp.is_conflict());

        r0.wait(timeout()).members(btreeset! {1,2}, "deleting inconsistent logs updates membership").await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(5000))
}
