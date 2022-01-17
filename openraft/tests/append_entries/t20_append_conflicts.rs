use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use memstore::ClientRequest;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::Entry;
use openraft::AppData;
use openraft::AppDataResponse;
use openraft::Config;
use openraft::LogId;
use openraft::MessageSummary;
use openraft::RaftStorage;
use openraft::State;

use crate::fixtures::blank;
use crate::fixtures::RaftRouter;

/// Test append-entries response in every case.
///
/// - bring up a learner and send to it append_entries request. Check the response in every case.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn append_conflicts() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let router = Arc::new(RaftRouter::new(config.clone()));
    router.new_raft_node(0).await;

    tracing::info!("--- wait for init node to ready");

    router.wait_for_log(&btreeset![0], None, timeout(), "empty").await?;
    router.wait_for_state(&btreeset![0], State::Learner, timeout(), "empty").await?;

    let (r0, sto0) = router.remove_node(0).await.unwrap();
    check_logs(&sto0, vec![]).await?;

    tracing::info!("--- case 0: prev_log_id == None, no logs");

    let req = AppendEntriesRequest {
        term: 1,
        leader_id: 0,
        prev_log_id: None,
        entries: vec![],
        leader_commit: Some(LogId::new(1, 2)),
    };

    let resp = r0.append_entries(req.clone()).await?;

    assert!(resp.success);
    assert!(!resp.conflict);

    tracing::info!("--- case 0: prev_log_id == None, 1 logs");

    let req = AppendEntriesRequest {
        term: 1,
        leader_id: 0,
        prev_log_id: None,
        entries: vec![blank(0, 0)],
        leader_commit: Some(LogId::new(1, 2)),
    };

    let resp = r0.append_entries(req.clone()).await?;
    assert!(resp.success);
    assert!(!resp.conflict);

    tracing::info!("--- case 0: prev_log_id == 1-1, 0 logs");

    let req = AppendEntriesRequest {
        term: 1,
        leader_id: 0,
        prev_log_id: Some(LogId::new(0, 0)),
        entries: vec![],
        leader_commit: Some(LogId::new(1, 2)),
    };

    let resp = r0.append_entries(req.clone()).await?;
    assert!(resp.success);
    assert!(!resp.conflict);

    check_logs(&sto0, vec![0]).await?;

    tracing::info!("--- case 0: prev_log_id.index == 0, ");

    let req = AppendEntriesRequest {
        term: 1,
        leader_id: 0,
        prev_log_id: Some(LogId::new(0, 0)),
        entries: vec![blank(1, 1), blank(1, 2), blank(1, 3), blank(1, 4)],
        // this set the last_applied to 2
        leader_commit: Some(LogId::new(1, 2)),
    };

    let resp = r0.append_entries(req.clone()).await?;
    assert!(resp.success);
    assert!(!resp.conflict);

    check_logs(&sto0, vec![0, 1, 1, 1, 1]).await?;

    tracing::info!("--- case 0: prev_log_id.index == 0, last_log_id mismatch");

    let resp = r0.append_entries(req.clone()).await?;
    assert!(resp.success);
    assert!(!resp.conflict);

    check_logs(&sto0, vec![0, 1, 1, 1, 1]).await?;

    // committed index is 2
    tracing::info!("--- case 1: 0 < prev_log_id.index < commit_index");

    let req = AppendEntriesRequest {
        term: 1,
        leader_id: 0,
        prev_log_id: Some(LogId::new(1, 1)),
        entries: vec![blank(1, 2)],
        leader_commit: Some(LogId::new(1, 2)),
    };

    let resp = r0.append_entries(req).await?;
    assert!(resp.success);
    assert!(!resp.conflict);

    check_logs(&sto0, vec![0, 1, 1, 1, 1]).await?;

    tracing::info!("--- case 2:  prev_log_id.index == last_applied, inconsistent log should be removed");

    let req = AppendEntriesRequest {
        term: 1,
        leader_id: 0,
        prev_log_id: Some(LogId::new(1, 2)),
        entries: vec![blank(2, 3)],
        // this set the last_applied to 2
        leader_commit: Some(LogId::new(1, 2)),
    };

    let resp = r0.append_entries(req).await?;
    assert!(resp.success);
    assert!(!resp.conflict);

    check_logs(&sto0, vec![0, 1, 1, 2]).await?;

    // check last_log_id is updated:
    let req = AppendEntriesRequest {
        term: 1,
        leader_id: 0,
        prev_log_id: Some(LogId::new(1, 2000)),
        entries: vec![],
        leader_commit: Some(LogId::new(1, 2)),
    };

    let resp = r0.append_entries(req).await?;
    assert!(!resp.success);
    assert!(resp.conflict);

    check_logs(&sto0, vec![0, 1, 1, 2]).await?;

    tracing::info!("--- case 3,4: prev_log_id.index <= last_log_id, prev_log_id mismatch, inconsistent log is removed");

    let req = AppendEntriesRequest {
        term: 1,
        leader_id: 0,
        prev_log_id: Some(LogId::new(3, 3)),
        entries: vec![],
        leader_commit: Some(LogId::new(1, 2)),
    };

    let resp = r0.append_entries(req).await?;
    assert!(!resp.success);
    assert!(resp.conflict);

    check_logs(&sto0, vec![0, 1, 1]).await?;

    tracing::info!("--- case 3,4: prev_log_id.index <= last_log_id, prev_log_id matches, inconsistent log is removed");
    // refill logs
    let req = AppendEntriesRequest {
        term: 1,
        leader_id: 0,
        prev_log_id: Some(LogId::new(1, 2)),
        entries: vec![blank(2, 3), blank(2, 4), blank(2, 5)],
        leader_commit: Some(LogId::new(1, 2)),
    };

    let resp = r0.append_entries(req).await?;
    assert!(resp.success);
    assert!(!resp.conflict);

    // check prepared store
    check_logs(&sto0, vec![0, 1, 1, 2, 2, 2]).await?;

    // prev_log_id matches
    let req = AppendEntriesRequest {
        term: 1,
        leader_id: 0,
        prev_log_id: Some(LogId::new(2, 3)),
        entries: vec![blank(3, 4)],
        leader_commit: Some(LogId::new(1, 2)),
    };

    let resp = r0.append_entries(req).await?;
    assert!(resp.success);
    assert!(!resp.conflict);

    check_logs(&sto0, vec![0, 1, 1, 2, 3]).await?;

    tracing::info!("--- case 5: last_log_id.index < prev_log_id.index");

    // refill logs
    let req = AppendEntriesRequest {
        term: 1,
        leader_id: 0,
        prev_log_id: Some(LogId::new(1, 200)),
        entries: vec![],
        leader_commit: Some(LogId::new(1, 2)),
    };

    let resp = r0.append_entries(req).await?;
    assert!(!resp.success);
    assert!(resp.conflict);

    Ok(())
}

/// To check if logs is as expected.
async fn check_logs<D, R, Sto>(sto: &Arc<Sto>, terms: Vec<u64>) -> Result<()>
where
    D: AppData,
    R: AppDataResponse,
    Sto: RaftStorage<D, R>,
{
    let logs = sto.get_log_entries(..).await?;
    let skip = 0;
    let want: Vec<Entry<ClientRequest>> = terms
        .iter()
        .skip(skip)
        .enumerate()
        .map(|(i, term)| blank(*term, (i + skip) as u64))
        .collect::<Vec<_>>();

    assert_eq!(want.as_slice().summary(), logs.as_slice().summary());

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
