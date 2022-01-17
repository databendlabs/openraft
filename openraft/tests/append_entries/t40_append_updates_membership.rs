use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::Entry;
use openraft::raft::EntryPayload;
use openraft::Config;
use openraft::LogId;
use openraft::Membership;
use openraft::State;

use crate::fixtures::blank;
use crate::fixtures::RaftRouter;

/// append-entries should update membership correctly when adding new logs and deleting
/// inconsistent logs.
///
/// - bring up a learner and send to it append_entries request. Check the membership updated.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn append_updates_membership() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let router = Arc::new(RaftRouter::new(config.clone()));
    router.new_raft_node(0).await;

    tracing::info!("--- wait for init node to ready");

    router.wait_for_log(&btreeset![0], None, None, "empty").await?;
    router.wait_for_state(&btreeset![0], State::Learner, None, "empty").await?;

    let (r0, _sto0) = router.remove_node(0).await.unwrap();

    tracing::info!("--- append-entries update membership");
    {
        let req = AppendEntriesRequest {
            term: 1,
            leader_id: 0,
            prev_log_id: None,
            entries: vec![
                blank(0, 0),
                blank(1, 1),
                Entry {
                    log_id: LogId { term: 1, index: 2 },
                    payload: EntryPayload::Membership(Membership::new_single(btreeset! {1,2})),
                },
                blank(1, 3),
                Entry {
                    log_id: LogId { term: 1, index: 4 },
                    payload: EntryPayload::Membership(Membership::new_single(btreeset! {1,2,3,4})),
                },
                blank(1, 5),
            ],
            leader_commit: Some(LogId::new(0, 0)),
        };

        let resp = r0.append_entries(req.clone()).await?;
        assert!(resp.success);
        assert!(!resp.conflict);

        r0.wait(timeout()).members(btreeset! {1,2,3,4}, "append-entries update membership").await?;
    }

    tracing::info!("--- delete inconsistent logs update membership");
    {
        let req = AppendEntriesRequest {
            term: 1,
            leader_id: 0,
            prev_log_id: Some(LogId::new(1, 2)),
            entries: vec![blank(2, 3)],
            leader_commit: Some(LogId::new(0, 0)),
        };

        let resp = r0.append_entries(req.clone()).await?;
        assert!(resp.success);
        assert!(!resp.conflict);

        r0.wait(timeout()).members(btreeset! {1,2}, "deleting inconsistent lgos updates membership").await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(5000))
}
