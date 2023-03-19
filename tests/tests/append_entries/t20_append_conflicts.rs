use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::raft::AppendEntriesRequest;
use openraft::CommittedLeaderId;
use openraft::Config;
use openraft::Entry;
use openraft::LogId;
use openraft::RaftStorage;
use openraft::RaftTypeConfig;
use openraft::ServerState;
use openraft::StorageHelper;
use openraft::Vote;

use crate::fixtures::blank;
use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Test append-entries response in every case.
///
/// - bring up a learner and send to it append_entries request. Check the response in every case.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
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

    router.wait_for_log(&btreeset![0], None, timeout(), "empty").await?;
    router.wait_for_state(&btreeset![0], ServerState::Learner, timeout(), "empty").await?;

    let (r0, mut sto0) = router.remove_node(0).unwrap();
    check_logs(&mut sto0, vec![]).await?;

    tracing::info!("--- case 0: prev_log_id == None, no logs");

    let req = AppendEntriesRequest {
        vote: Vote::new_committed(1, 2),
        prev_log_id: None,
        entries: vec![],
        leader_commit: Some(LogId::new(CommittedLeaderId::new(1, 0), 2)),
    };

    let resp = r0.append_entries(req.clone()).await?;

    assert!(resp.is_success());
    assert!(!resp.is_conflict());

    tracing::info!("--- case 0: prev_log_id == None, 1 logs");

    let req = AppendEntriesRequest {
        vote: Vote::new_committed(1, 2),
        prev_log_id: None,
        entries: vec![blank(0, 0)],
        leader_commit: Some(LogId::new(CommittedLeaderId::new(1, 0), 2)),
    };

    let resp = r0.append_entries(req.clone()).await?;
    assert!(resp.is_success());
    assert!(!resp.is_conflict());

    tracing::info!("--- case 0: prev_log_id == 1-1, 0 logs");

    let req = AppendEntriesRequest {
        vote: Vote::new_committed(1, 2),
        prev_log_id: Some(LogId::new(CommittedLeaderId::new(0, 0), 0)),
        entries: vec![],
        leader_commit: Some(LogId::new(CommittedLeaderId::new(1, 0), 2)),
    };

    let resp = r0.append_entries(req.clone()).await?;
    assert!(resp.is_success());
    assert!(!resp.is_conflict());

    check_logs(&mut sto0, vec![0]).await?;

    tracing::info!("--- case 0: prev_log_id.index == 0, ");

    let req = AppendEntriesRequest {
        vote: Vote::new_committed(1, 2),
        prev_log_id: Some(LogId::new(CommittedLeaderId::new(0, 0), 0)),
        entries: vec![blank(1, 1), blank(1, 2), blank(1, 3), blank(1, 4)],
        // this set the last_applied to 2
        leader_commit: Some(LogId::new(CommittedLeaderId::new(1, 0), 2)),
    };

    let resp = r0.append_entries(req.clone()).await?;
    assert!(resp.is_success());
    assert!(!resp.is_conflict());

    check_logs(&mut sto0, vec![0, 1, 1, 1, 1]).await?;

    tracing::info!("--- case 0: prev_log_id.index == 0, last_log_id mismatch");

    let resp = r0.append_entries(req.clone()).await?;
    assert!(resp.is_success());
    assert!(!resp.is_conflict());

    check_logs(&mut sto0, vec![0, 1, 1, 1, 1]).await?;

    // committed index is 2
    tracing::info!("--- case 1: 0 < prev_log_id.index < commit_index");

    let req = AppendEntriesRequest {
        vote: Vote::new_committed(1, 2),
        prev_log_id: Some(LogId::new(CommittedLeaderId::new(1, 0), 1)),
        entries: vec![blank(1, 2)],
        leader_commit: Some(LogId::new(CommittedLeaderId::new(1, 0), 2)),
    };

    let resp = r0.append_entries(req).await?;
    assert!(resp.is_success());
    assert!(!resp.is_conflict());

    check_logs(&mut sto0, vec![0, 1, 1, 1, 1]).await?;

    tracing::info!("--- case 2:  prev_log_id.index == last_applied, inconsistent log should be removed");

    let req = AppendEntriesRequest {
        vote: Vote::new_committed(1, 2),
        prev_log_id: Some(LogId::new(CommittedLeaderId::new(1, 0), 2)),
        entries: vec![blank(2, 3)],
        // this set the last_applied to 2
        leader_commit: Some(LogId::new(CommittedLeaderId::new(1, 0), 2)),
    };

    let resp = r0.append_entries(req).await?;
    assert!(resp.is_success());
    assert!(!resp.is_conflict());

    check_logs(&mut sto0, vec![0, 1, 1, 2]).await?;

    // check last_log_id is updated:
    let req = AppendEntriesRequest {
        vote: Vote::new_committed(1, 2),
        prev_log_id: Some(LogId::new(CommittedLeaderId::new(1, 0), 2000)),
        entries: vec![],
        leader_commit: Some(LogId::new(CommittedLeaderId::new(1, 0), 2)),
    };

    let resp = r0.append_entries(req).await?;
    assert!(!resp.is_success());
    assert!(resp.is_conflict());

    check_logs(&mut sto0, vec![0, 1, 1, 2]).await?;

    tracing::info!("--- case 3,4: prev_log_id.index <= last_log_id, prev_log_id mismatch, inconsistent log is removed");

    let req = AppendEntriesRequest {
        vote: Vote::new_committed(1, 2),
        prev_log_id: Some(LogId::new(CommittedLeaderId::new(3, 0), 3)),
        entries: vec![],
        leader_commit: Some(LogId::new(CommittedLeaderId::new(1, 0), 2)),
    };

    let resp = r0.append_entries(req).await?;
    assert!(!resp.is_success());
    assert!(resp.is_conflict());

    check_logs(&mut sto0, vec![0, 1, 1]).await?;

    tracing::info!("--- case 3,4: prev_log_id.index <= last_log_id, prev_log_id matches, inconsistent log is removed");
    // refill logs
    let req = AppendEntriesRequest {
        vote: Vote::new_committed(1, 2),
        prev_log_id: Some(LogId::new(CommittedLeaderId::new(1, 0), 2)),
        entries: vec![blank(2, 3), blank(2, 4), blank(2, 5)],
        leader_commit: Some(LogId::new(CommittedLeaderId::new(1, 0), 2)),
    };

    let resp = r0.append_entries(req).await?;
    assert!(resp.is_success());
    assert!(!resp.is_conflict());

    // check prepared store
    check_logs(&mut sto0, vec![0, 1, 1, 2, 2, 2]).await?;

    // prev_log_id matches
    let req = AppendEntriesRequest {
        vote: Vote::new_committed(1, 2),
        prev_log_id: Some(LogId::new(CommittedLeaderId::new(2, 0), 3)),
        entries: vec![blank(3, 4)],
        leader_commit: Some(LogId::new(CommittedLeaderId::new(1, 0), 2)),
    };

    let resp = r0.append_entries(req).await?;
    assert!(resp.is_success());
    assert!(!resp.is_conflict());

    check_logs(&mut sto0, vec![0, 1, 1, 2, 3]).await?;

    tracing::info!("--- case 5: last_log_id.index < prev_log_id.index");

    // refill logs
    let req = AppendEntriesRequest {
        vote: Vote::new_committed(1, 2),
        prev_log_id: Some(LogId::new(CommittedLeaderId::new(1, 0), 200)),
        entries: vec![],
        leader_commit: Some(LogId::new(CommittedLeaderId::new(1, 0), 2)),
    };

    let resp = r0.append_entries(req).await?;
    assert!(!resp.is_success());
    assert!(resp.is_conflict());

    Ok(())
}

/// To check if logs is as expected.
async fn check_logs<C, Sto>(sto: &mut Sto, terms: Vec<u64>) -> Result<()>
where
    C: RaftTypeConfig,
    Sto: RaftStorage<C>,
{
    let logs = StorageHelper::new(sto).get_log_entries(..).await?;
    let skip = 0;
    let want: Vec<Entry<openraft_memstore::Config>> = terms
        .iter()
        .skip(skip)
        .enumerate()
        .map(|(i, term)| blank(*term, (i + skip) as u64))
        .collect::<Vec<_>>();

    let w = format!("{:?}", &want);
    let g = format!("{:?}", &logs);

    assert_eq!(w, g);

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
