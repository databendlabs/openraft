use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use fixtures::RaftRouter;
use maplit::btreeset;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::Entry;
use openraft::raft::EntryPayload;
use openraft::raft::Membership;
use openraft::AppData;
use openraft::Config;
use openraft::LogId;
use openraft::State;

#[macro_use]
mod fixtures;

/// append-entries should update membership correctly when adding new logs and deleting
/// inconsistent logs.
///
/// - bring up a non-voter and send to it append_entries request. Check the membership updated.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn append_updates_membership() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let router = Arc::new(RaftRouter::new(config.clone()));
    router.new_raft_node(0).await;

    let n_logs = 0;

    tracing::info!("--- wait for init node to ready");

    router.wait_for_log(&btreeset![0], n_logs, None, "empty").await?;
    router.wait_for_state(&btreeset![0], State::Learner, None, "empty").await?;

    let (r0, _sto0) = router.remove_node(0).await.unwrap();

    tracing::info!("--- append-entries update membership");
    {
        let req = AppendEntriesRequest {
            term: 1,
            leader_id: 0,
            prev_log_id: LogId::new(0, 0),
            entries: vec![
                ent(1, 1),
                Entry {
                    log_id: LogId { term: 1, index: 2 },
                    payload: EntryPayload::Membership(Membership::new_single(btreeset! {1,2})),
                },
                ent(1, 3),
                Entry {
                    log_id: LogId { term: 1, index: 4 },
                    payload: EntryPayload::Membership(Membership::new_single(btreeset! {1,2,3,4})),
                },
                ent(1, 5),
            ],
            leader_commit: LogId::new(0, 0),
        };

        let resp = r0.append_entries(req.clone()).await?;
        assert!(resp.success());
        assert_eq!(None, resp.conflict);

        r0.wait(timeout()).members(btreeset! {1,2,3,4}, "append-entries update membership").await?;
    }

    tracing::info!("--- delete inconsistent logs update membership");
    {
        let req = AppendEntriesRequest {
            term: 1,
            leader_id: 0,
            prev_log_id: LogId::new(1, 2),
            entries: vec![ent(2, 3)],
            leader_commit: LogId::new(0, 0),
        };

        let resp = r0.append_entries(req.clone()).await?;
        assert!(resp.success());
        assert_eq!(None, resp.conflict);

        r0.wait(timeout()).members(btreeset! {1,2}, "deleting inconsistent lgos updates membership").await?;
    }

    Ok(())
}

/// Create a blonk log entry for test.
fn ent<T: AppData>(term: u64, index: u64) -> Entry<T> {
    Entry {
        log_id: LogId { term, index },
        payload: EntryPayload::Blank,
    }
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(5000))
}
