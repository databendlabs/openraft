use std::sync::Arc;

use anyhow::Result;
use async_raft::raft::AppendEntriesRequest;
use async_raft::raft::ConflictOpt;
use async_raft::raft::Entry;
use async_raft::raft::EntryPayload;
use async_raft::Config;
use async_raft::LogId;
use async_raft::RaftNetwork;
use fixtures::RaftRouter;
use memstore::ClientRequest;

#[macro_use]
mod fixtures;

/// Cluster conflict_with_empty_entries test.
///
/// `append_entries` should get a response with non-none ConflictOpt even if the entries in message
/// is empty.
/// Otherwise if no conflict is found the leader will never be able to sync logs to a new added
/// NonVoter, until a next log is proposed on leader.
///
/// What does this test do?
///
/// - brings a 1 NonVoter node online.
///
/// - send `append_logs` message to it with empty `entries` and some non-zero `prev_log_index`.
/// - asserts that a response with ConflictOpt set.
///
/// - feed several logs to it.
///
/// - send `append_logs` message with conflicting prev_log_index and empty `entries`.
/// - asserts that a response with ConflictOpt set.
///
/// RUST_LOG=async_raft,memstore,conflict_with_empty_entries=trace cargo test -p async-raft --test
/// conflict_with_empty_entries
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn conflict_with_empty_entries() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let router = Arc::new(RaftRouter::new(config.clone()));

    router.new_raft_node(0).await;

    // Expect conflict even if the message contains no entries.

    let rpc = AppendEntriesRequest::<memstore::ClientRequest> {
        term: 1,
        leader_id: 1,
        prev_log_id: LogId::new(1, 5),
        entries: vec![],
        leader_commit: 5,
    };

    let resp = router.send_append_entries(0, rpc).await?;
    assert!(!resp.success);
    assert!(resp.conflict_opt.is_some());
    let c = resp.conflict_opt.unwrap();
    assert_eq!(
        ConflictOpt {
            log_id: LogId { term: 1, index: 5 }
        },
        c
    );

    // Feed 2 logs

    let rpc = AppendEntriesRequest::<memstore::ClientRequest> {
        term: 1,
        leader_id: 1,
        prev_log_id: LogId::new(0, 0),
        entries: vec![
            Entry {
                log_id: (1, 1).into(),
                payload: EntryPayload::Blank,
            },
            Entry {
                log_id: (1, 2).into(),
                payload: EntryPayload::Normal(ClientRequest {
                    client: "foo".to_string(),
                    serial: 1,
                    status: "bar".to_string(),
                }),
            },
        ],
        leader_commit: 5,
    };

    let resp = router.send_append_entries(0, rpc).await?;
    assert!(resp.success);
    assert!(resp.conflict_opt.is_none());

    // Expect a conflict with prev_log_index == 3

    let rpc = AppendEntriesRequest::<memstore::ClientRequest> {
        term: 1,
        leader_id: 1,
        prev_log_id: LogId::new(1, 3),
        entries: vec![],
        leader_commit: 5,
    };

    let resp = router.send_append_entries(0, rpc).await?;
    assert!(!resp.success);
    assert!(resp.conflict_opt.is_some());
    let c = resp.conflict_opt.unwrap();
    assert_eq!(
        ConflictOpt {
            log_id: LogId { term: 1, index: 3 }
        },
        c
    );

    Ok(())
}
