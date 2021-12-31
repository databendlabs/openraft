use std::sync::Arc;

use anyhow::Result;
use async_raft::raft::AppendEntriesRequest;
use async_raft::raft::Entry;
use async_raft::AppData;
use async_raft::AppDataResponse;
use async_raft::Config;
use async_raft::LogId;
use async_raft::MessageSummary;
use async_raft::RaftStorage;
use async_raft::State;
use fixtures::RaftRouter;
use maplit::btreeset;
use memstore::ClientRequest;

use crate::fixtures::ent;

#[macro_use]
mod fixtures;

/// Test append-entries response in every case.
///
/// - bring up a non-voter and send to it append_entries request. Check the response in every case.
///
/// RUST_LOG=async_raft,memstore,append_conflicts=trace cargo test -p async-raft --test append_conflicts
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn append_conflicts() -> Result<()> {
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

    let (r0, sto0) = router.remove_node(0).await.unwrap();
    check_logs(&sto0, vec![0]).await?;

    tracing::info!("--- case 0: prev_log_id.index == 0, no logs");

    let req = AppendEntriesRequest {
        term: 1,
        leader_id: 0,
        prev_log_id: LogId::new(0, 0),
        entries: vec![],
        leader_commit: LogId::new(1, 2),
    };

    let resp = r0.append_entries(req.clone()).await?;
    assert!(resp.success());
    assert_eq!(None, resp.conflict);

    check_logs(&sto0, vec![0]).await?;

    tracing::info!("--- case 0: prev_log_id.index == 0, ");

    let req = AppendEntriesRequest {
        term: 1,
        leader_id: 0,
        prev_log_id: LogId::new(0, 0),
        entries: vec![ent(1, 1), ent(1, 2), ent(1, 3), ent(1, 4)],
        // this set the last_applied to 2
        leader_commit: LogId::new(1, 2),
    };

    let resp = r0.append_entries(req.clone()).await?;
    assert!(resp.success());
    assert_eq!(None, resp.conflict);

    check_logs(&sto0, vec![0, 1, 1, 1, 1]).await?;

    tracing::info!("--- case 0: prev_log_id.index == 0, last_log_id mismatch");

    let resp = r0.append_entries(req.clone()).await?;
    assert!(resp.success());
    assert_eq!(None, resp.conflict);

    check_logs(&sto0, vec![0, 1, 1, 1, 1]).await?;

    // committed index is 2
    tracing::info!("--- case 1: 0 < prev_log_id.index < commit_index");

    let req = AppendEntriesRequest {
        term: 1,
        leader_id: 0,
        prev_log_id: LogId::new(1, 1),
        entries: vec![ent(1, 2)],
        leader_commit: LogId::new(1, 2),
    };

    let resp = r0.append_entries(req).await?;
    assert!(resp.success());
    assert_eq!(None, resp.conflict);

    check_logs(&sto0, vec![0, 1, 1, 1, 1]).await?;

    tracing::info!("--- case 2:  prev_log_id.index == last_applied, inconsistent log should be removed");

    let req = AppendEntriesRequest {
        term: 1,
        leader_id: 0,
        prev_log_id: LogId::new(1, 2),
        entries: vec![ent(2, 3)],
        // this set the last_applied to 2
        leader_commit: LogId::new(1, 2),
    };

    let resp = r0.append_entries(req).await?;
    assert!(resp.success());
    assert_eq!(None, resp.conflict);

    check_logs(&sto0, vec![0, 1, 1, 2]).await?;

    // check last_log_id is updated:
    let req = AppendEntriesRequest {
        term: 1,
        leader_id: 0,
        prev_log_id: LogId::new(1, 2000),
        entries: vec![],
        leader_commit: LogId::new(1, 2),
    };

    let resp = r0.append_entries(req).await?;
    assert!(!resp.success());
    assert_eq!(Some(LogId::new(1, 2000)), resp.conflict);

    check_logs(&sto0, vec![0, 1, 1, 2]).await?;

    tracing::info!("--- case 3,4: prev_log_id.index <= last_log_id, prev_log_id mismatch, inconsistent log is removed");

    let req = AppendEntriesRequest {
        term: 1,
        leader_id: 0,
        prev_log_id: LogId::new(3, 3),
        entries: vec![],
        leader_commit: LogId::new(1, 2),
    };

    let resp = r0.append_entries(req).await?;
    assert!(!resp.success());
    // returns the id just before prev_log_id.index
    assert_eq!(Some(LogId::new(3, 3)), resp.conflict);

    check_logs(&sto0, vec![0, 1, 1]).await?;

    tracing::info!("--- case 3,4: prev_log_id.index <= last_log_id, prev_log_id matches, inconsistent log is removed");
    // refill logs
    let req = AppendEntriesRequest {
        term: 1,
        leader_id: 0,
        prev_log_id: LogId::new(1, 2),
        entries: vec![ent(2, 3), ent(2, 4), ent(2, 5)],
        leader_commit: LogId::new(1, 2),
    };

    let resp = r0.append_entries(req).await?;
    assert!(resp.success());
    assert_eq!(None, resp.conflict);

    // check prepared store
    check_logs(&sto0, vec![0, 1, 1, 2, 2, 2]).await?;

    // prev_log_id matches
    let req = AppendEntriesRequest {
        term: 1,
        leader_id: 0,
        prev_log_id: LogId::new(2, 3),
        entries: vec![ent(3, 4)],
        leader_commit: LogId::new(1, 2),
    };

    let resp = r0.append_entries(req).await?;
    assert!(resp.success());
    assert_eq!(None, resp.conflict);

    check_logs(&sto0, vec![0, 1, 1, 2, 3]).await?;

    tracing::info!("--- case 5: last_log_id.index < prev_log_id.index");

    // refill logs
    let req = AppendEntriesRequest {
        term: 1,
        leader_id: 0,
        prev_log_id: LogId::new(1, 200),
        entries: vec![],
        leader_commit: LogId::new(1, 2),
    };

    let resp = r0.append_entries(req).await?;
    assert!(!resp.success());
    assert_eq!(Some(LogId::new(1, 200)), resp.conflict);

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
        .map(|(i, term)| ent(*term, (i + skip) as u64))
        .collect::<Vec<_>>();

    assert_eq!(want.as_slice().summary(), logs.as_slice().summary());

    Ok(())
}
