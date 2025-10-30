use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use openraft::Config;
use openraft::Entry;
use openraft::RaftLogReader;
use openraft::RaftTypeConfig;
use openraft::ServerState;
use openraft::Vote;
use openraft::raft::AppendEntriesRequest;
use openraft::storage::RaftLogStorage;
use openraft::testing::blank_ent;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// Test append-entries response in every case.
///
/// - bring up a learner and send to it append_entries request. Check the response in every case.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn append_conflicts() -> Result<()> {
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

    router.wait(&0, timeout()).applied_index(None, "empty").await?;
    router.wait(&0, timeout()).state(ServerState::Learner, "empty").await?;

    let (r0, mut sto0, _sm0) = router.remove_node(0).unwrap();
    check_logs(&mut sto0, vec![]).await?;

    tracing::info!("--- case 0: prev_log_id == None, no logs");

    let req = AppendEntriesRequest {
        vote: Vote::new_committed(1, 2),
        prev_log_id: None,
        entries: vec![],
        leader_commit: Some(log_id(1, 0, 2)),
    };

    let resp = r0.append_entries(req).await?;

    assert!(resp.is_success());
    assert!(!resp.is_conflict());

    tracing::info!("--- case 0: prev_log_id == None, 1 logs");

    let req = AppendEntriesRequest {
        vote: Vote::new_committed(1, 2),
        prev_log_id: None,
        entries: vec![blank_ent(0, 0, 0)],
        leader_commit: Some(log_id(1, 0, 2)),
    };

    let resp = r0.append_entries(req).await?;
    assert!(resp.is_success());
    assert!(!resp.is_conflict());

    tracing::info!("--- case 0: prev_log_id == 1-1, 0 logs");

    let req = AppendEntriesRequest {
        vote: Vote::new_committed(1, 2),
        prev_log_id: Some(log_id(0, 0, 0)),
        entries: vec![],
        leader_commit: Some(log_id(1, 0, 2)),
    };

    let resp = r0.append_entries(req).await?;
    assert!(resp.is_success());
    assert!(!resp.is_conflict());

    check_logs(&mut sto0, vec![0]).await?;

    tracing::info!("--- case 0: prev_log_id.index == 0, ");

    let req = || AppendEntriesRequest {
        vote: Vote::new_committed(1, 2),
        prev_log_id: Some(log_id(0, 0, 0)),
        entries: vec![
            blank_ent(1, 0, 1),
            blank_ent(1, 0, 2),
            blank_ent(1, 0, 3),
            blank_ent(1, 0, 4),
        ],
        // this set the last_applied to 2
        leader_commit: Some(log_id(1, 0, 2)),
    };

    let resp = r0.append_entries(req()).await?;
    assert!(resp.is_success());
    assert!(!resp.is_conflict());

    check_logs(&mut sto0, vec![0, 1, 1, 1, 1]).await?;

    tracing::info!("--- case 0: prev_log_id.index == 0, last_log_id mismatch");

    let resp = r0.append_entries(req()).await?;
    assert!(resp.is_success());
    assert!(!resp.is_conflict());

    check_logs(&mut sto0, vec![0, 1, 1, 1, 1]).await?;

    // committed index is 2
    tracing::info!("--- case 1: 0 < prev_log_id.index < commit_index");

    let req = AppendEntriesRequest {
        vote: Vote::new_committed(1, 2),
        prev_log_id: Some(log_id(1, 0, 1)),
        entries: vec![blank_ent(1, 0, 2)],
        leader_commit: Some(log_id(1, 0, 2)),
    };

    let resp = r0.append_entries(req).await?;
    assert!(resp.is_success());
    assert!(!resp.is_conflict());

    check_logs(&mut sto0, vec![0, 1, 1, 1, 1]).await?;

    tracing::info!("--- case 2:  prev_log_id.index == last_applied, inconsistent log should be removed");

    let req = AppendEntriesRequest {
        vote: Vote::new_committed(1, 2),
        prev_log_id: Some(log_id(1, 0, 2)),
        entries: vec![blank_ent(2, 0, 3)],
        // this set the last_applied to 2
        leader_commit: Some(log_id(1, 0, 2)),
    };

    let resp = r0.append_entries(req).await?;
    assert!(resp.is_success());
    assert!(!resp.is_conflict());

    check_logs(&mut sto0, vec![0, 1, 1, 2]).await?;

    // check last_log_id is updated:
    let req = AppendEntriesRequest {
        vote: Vote::new_committed(1, 2),
        prev_log_id: Some(log_id(1, 0, 2000)),
        entries: vec![],
        leader_commit: Some(log_id(1, 0, 2)),
    };

    let resp = r0.append_entries(req).await?;
    assert!(!resp.is_success());
    assert!(resp.is_conflict());

    check_logs(&mut sto0, vec![0, 1, 1, 2]).await?;

    tracing::info!("--- case 3,4: prev_log_id.index <= last_log_id, prev_log_id mismatch, inconsistent log is removed");

    let req = AppendEntriesRequest {
        vote: Vote::new_committed(1, 2),
        prev_log_id: Some(log_id(3, 0, 3)),
        entries: vec![],
        leader_commit: Some(log_id(1, 0, 2)),
    };

    let resp = r0.append_entries(req).await?;
    assert!(!resp.is_success());
    assert!(resp.is_conflict());

    check_logs(&mut sto0, vec![0, 1, 1]).await?;

    tracing::info!("--- case 3,4: prev_log_id.index <= last_log_id, prev_log_id matches, inconsistent log is removed");
    // refill logs
    let req = AppendEntriesRequest {
        vote: Vote::new_committed(1, 2),
        prev_log_id: Some(log_id(1, 0, 2)),
        entries: vec![blank_ent(2, 0, 3), blank_ent(2, 0, 4), blank_ent(2, 0, 5)],
        leader_commit: Some(log_id(1, 0, 2)),
    };

    let resp = r0.append_entries(req).await?;
    assert!(resp.is_success());
    assert!(!resp.is_conflict());

    // check prepared store
    check_logs(&mut sto0, vec![0, 1, 1, 2, 2, 2]).await?;

    // prev_log_id matches
    let req = AppendEntriesRequest {
        vote: Vote::new_committed(1, 2),
        prev_log_id: Some(log_id(2, 0, 3)),
        entries: vec![blank_ent(3, 0, 4)],
        leader_commit: Some(log_id(1, 0, 2)),
    };

    let resp = r0.append_entries(req).await?;
    assert!(resp.is_success());
    assert!(!resp.is_conflict());

    check_logs(&mut sto0, vec![0, 1, 1, 2, 3]).await?;

    tracing::info!("--- case 5: last_log_id.index < prev_log_id.index");

    // refill logs
    let req = AppendEntriesRequest {
        vote: Vote::new_committed(1, 2),
        prev_log_id: Some(log_id(1, 0, 200)),
        entries: vec![],
        leader_commit: Some(log_id(1, 0, 2)),
    };

    let resp = r0.append_entries(req).await?;
    assert!(!resp.is_success());
    assert!(resp.is_conflict());

    Ok(())
}

/// To check if logs is as expected.
async fn check_logs<C, LS>(log_store: &mut LS, terms: Vec<u64>) -> Result<()>
where
    C: RaftTypeConfig,
    LS: RaftLogStorage<C>,
{
    let logs = log_store.get_log_reader().await.try_get_log_entries(..).await?;
    let skip = 0;
    let want: Vec<Entry<openraft_memstore::TypeConfig>> = terms
        .iter()
        .skip(skip)
        .enumerate()
        .map(|(i, term)| blank_ent(*term, 0, (i + skip) as u64))
        .collect::<Vec<_>>();

    let w = format!("{:?}", &want);
    let g = format!("{:?}", &logs);

    assert_eq!(w, g);

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
