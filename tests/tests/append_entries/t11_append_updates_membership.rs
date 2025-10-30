use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::Entry;
use openraft::EntryPayload;
use openraft::Membership;
use openraft::ServerState;
use openraft::Vote;
use openraft::raft::AppendEntriesRequest;
use openraft::testing::blank_ent;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// append-entries should update membership correctly when adding new logs and deleting
/// inconsistent logs.
///
/// - bring up a learner and send to it append_entries request. Check the membership updated.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
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

    router.wait(&0, None).applied_index(None, "empty").await?;
    router.wait(&0, None).state(ServerState::Learner, "empty").await?;

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
                    log_id: log_id(1, 0, 2),
                    payload: EntryPayload::Membership(Membership::new_with_defaults(vec![btreeset! {1,2}], [])),
                },
                blank_ent(1, 0, 3),
                Entry {
                    log_id: log_id(1, 0, 4),
                    payload: EntryPayload::Membership(Membership::new_with_defaults(vec![btreeset! {1,2,3,4}], [])),
                },
                blank_ent(1, 0, 5),
            ],
            leader_commit: Some(log_id(0, 0, 0)),
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
            prev_log_id: Some(log_id(1, 0, 2)),
            entries: vec![blank_ent(2, 0, 3)],
            leader_commit: Some(log_id(0, 0, 0)),
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
