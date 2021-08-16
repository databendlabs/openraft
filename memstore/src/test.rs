use std::collections::BTreeSet;

use async_raft::raft::EntryConfigChange;
use async_raft::raft::EntryNormal;

use super::*;

const NODE_ID: u64 = 0;

//////////////////////////////////////////////////////////////////////////////

#[tokio::test]
async fn test_get_membership_config_default() -> Result<()> {
    let store = MemStore::new(NODE_ID);
    let membership = store.get_membership_config().await?;
    assert_eq!(membership.members.len(), 1, "expected members len of 1");
    assert!(
        membership.members_after_consensus.is_none(),
        "expected None for default members_after_consensus"
    );
    Ok(())
}

#[tokio::test]
async fn test_get_membership_config_with_previous_state() -> Result<()> {
    let mut log = BTreeMap::new();
    let mut members: BTreeSet<NodeId> = Default::default();
    members.insert(1);
    members.insert(2);
    members.insert(3);
    log.insert(1, Entry {
        log_id: (1, 1).into(),
        payload: EntryPayload::ConfigChange(EntryConfigChange {
            membership: MembershipConfig {
                members: members.clone(),
                members_after_consensus: None,
            },
        }),
    });
    let sm = MemStoreStateMachine::default();
    let hs = HardState {
        current_term: 1,
        voted_for: Some(NODE_ID),
    };
    let store = MemStore::new_with_state(NODE_ID, log, sm, Some(hs.clone()), None);

    let initial = store.get_membership_config().await?;

    assert_eq!(&initial.members, &members, "unexpected len for members");
    assert!(
        initial.members_after_consensus.is_none(),
        "unexpected value for members_after_consensus"
    );
    Ok(())
}

//////////////////////////////////////////////////////////////////////////////

#[tokio::test]
async fn test_get_initial_state_default() -> Result<()> {
    let store = MemStore::new(NODE_ID);
    let expected_hs = HardState {
        current_term: 0,
        voted_for: None,
    };
    let expected_membership = MembershipConfig::new_initial(NODE_ID);

    let initial = store.get_initial_state().await?;

    assert_eq!(
        initial.last_log_id,
        (0, 0).into(),
        "unexpected default value for last log"
    );
    assert_eq!(
        initial.last_applied_log,
        LogId { term: 0, index: 0 },
        "unexpected value for last applied log"
    );
    assert_eq!(
        initial.hard_state, expected_hs,
        "unexpected value for default hard state"
    );
    assert_eq!(
        initial.membership, expected_membership,
        "unexpected value for default membership config"
    );
    Ok(())
}

#[tokio::test]
async fn test_get_initial_state_with_previous_state() -> Result<()> {
    let mut log = BTreeMap::new();
    log.insert(1, Entry {
        log_id: (1, 1).into(),
        payload: EntryPayload::Blank,
    });
    let sm = MemStoreStateMachine {
        last_applied_log: LogId { term: 3, index: 1 }, // Just stubbed in for testing.
        ..Default::default()
    };
    let hs = HardState {
        current_term: 1,
        voted_for: Some(NODE_ID),
    };
    let store = MemStore::new_with_state(NODE_ID, log, sm, Some(hs.clone()), None);

    let initial = store.get_initial_state().await?;

    assert_eq!(
        initial.last_log_id,
        (1, 1).into(),
        "unexpected default value for last log"
    );
    assert_eq!(
        initial.last_applied_log,
        LogId { term: 3, index: 1 },
        "unexpected value for last applied log"
    );
    assert_eq!(initial.hard_state, hs, "unexpected value for default hard state");
    Ok(())
}

//////////////////////////////////////////////////////////////////////////////

#[tokio::test]
async fn test_save_hard_state() -> Result<()> {
    let store = MemStore::new(NODE_ID);
    let new_hs = HardState {
        current_term: 100,
        voted_for: Some(NODE_ID),
    };

    let initial = store.get_initial_state().await?;
    store.save_hard_state(&new_hs).await?;
    let post = store.get_initial_state().await?;

    assert_ne!(
        initial.hard_state, post.hard_state,
        "hard state was expected to be different after update"
    );
    Ok(())
}

//////////////////////////////////////////////////////////////////////////////

#[tokio::test]
async fn test_get_log_entries_returns_emptry_vec_when_start_gt_stop() -> Result<()> {
    let store = default_store_with_logs();

    let logs = store.get_log_entries(10, 1).await?;

    assert_eq!(logs.len(), 0, "expected no logs to be returned");
    Ok(())
}

#[tokio::test]
async fn test_get_log_entries_returns_expected_entries() -> Result<()> {
    let store = default_store_with_logs();

    let logs = store.get_log_entries(5, 7).await?;

    assert_eq!(logs.len(), 2, "expected two logs to be returned");
    assert_eq!(logs[0].log_id, (1, 5).into(), "unexpected value for log id");
    assert_eq!(logs[1].log_id, (1, 6).into(), "unexpected value for log id");
    Ok(())
}

//////////////////////////////////////////////////////////////////////////////

#[tokio::test]
async fn test_delete_logs_from_does_nothing_if_start_gt_stop() -> Result<()> {
    let store = default_store_with_logs();

    store.delete_logs_from(10, Some(1)).await?;
    let logs = store.get_log_entries(1, 11).await?;

    assert_eq!(logs.len(), 10, "expected all (10) logs to be preserved");
    Ok(())
}

#[tokio::test]
async fn test_delete_logs_from_deletes_target_logs() -> Result<()> {
    let store = default_store_with_logs();

    store.delete_logs_from(1, Some(11)).await?;
    let logs = store.get_log_entries(0, 100).await?;

    assert_eq!(logs.len(), 0, "expected all logs to be deleted");
    Ok(())
}

#[tokio::test]
async fn test_delete_logs_from_deletes_target_logs_no_stop() -> Result<()> {
    let store = default_store_with_logs();

    store.delete_logs_from(1, None).await?;
    let logs = store.get_log_entries(0, 100).await?;

    assert_eq!(logs.len(), 0, "expected all logs to be deleted");
    Ok(())
}

#[tokio::test]
async fn test_delete_logs_from_deletes_only_target_logs() -> Result<()> {
    let store = default_store_with_logs();

    store.delete_logs_from(1, Some(10)).await?;
    let logs = store.get_log_entries(0, 100).await?;

    assert_eq!(logs.len(), 1, "expected one log to be preserved");
    assert_eq!(logs[0].log_id.index, 10, "unexpected log index");
    Ok(())
}

//////////////////////////////////////////////////////////////////////////////

#[tokio::test]
async fn test_append_entry_to_log() -> Result<()> {
    let store = default_store_with_logs();

    store
        .append_entry_to_log(&Entry {
            log_id: (2, 10).into(),
            payload: EntryPayload::Blank,
        })
        .await?;
    let log = store.get_log().await;

    assert_eq!(log.len(), 10, "expected 10 entries to exist in the log");
    assert_eq!(log[&10].log_id, (2, 10).into(), "unexpected log id");
    Ok(())
}

//////////////////////////////////////////////////////////////////////////////

#[tokio::test]
async fn test_replicate_to_log() -> Result<()> {
    let store = default_store_with_logs();

    store
        .replicate_to_log(&[Entry {
            log_id: (1, 11).into(),
            payload: EntryPayload::Blank,
        }])
        .await?;
    let log = store.get_log().await;

    assert_eq!(log.len(), 11, "expected 11 entries to exist in the log");
    assert_eq!(log[&11].log_id, (1, 11).into(), "unexpected log id");
    Ok(())
}

//////////////////////////////////////////////////////////////////////////////

#[tokio::test]
async fn test_apply_entry_to_state_machine() -> Result<()> {
    let store = default_store_with_logs();

    let entry = Entry {
        log_id: LogId { term: 3, index: 1 },

        payload: EntryPayload::Normal(EntryNormal {
            data: ClientRequest {
                client: "0".into(),
                serial: 0,
                status: "lit".into(),
            },
        }),
    };
    store.apply_entry_to_state_machine(&entry).await?;
    let sm = store.get_state_machine().await;

    assert_eq!(
        sm.last_applied_log,
        LogId { term: 3, index: 1 },
        "expected last_applied_log to be 1, got {}",
        sm.last_applied_log
    );
    let client_serial =
        sm.client_serial_responses.get("0").expect("expected entry to exist in client_serial_responses");
    assert_eq!(client_serial.0, 0, "unexpected client serial response");
    assert_eq!(client_serial.1, None, "unexpected client serial response");
    let client_status = sm.client_status.get("0").expect("expected entry to exist in client_status");
    assert_eq!(
        client_status, "lit",
        "expected client_status to be 'lit', got '{}'",
        client_status
    );
    Ok(())
}

//////////////////////////////////////////////////////////////////////////////

#[tokio::test]
async fn test_replicate_to_state_machine() -> Result<()> {
    let store = default_store_with_logs();

    let req0 = ClientRequest {
        client: "1".into(),
        serial: 0,
        status: "old".into(),
    };
    let req1 = ClientRequest {
        client: "1".into(),
        serial: 1,
        status: "new".into(),
    };
    let req2 = ClientRequest {
        client: "2".into(),
        serial: 0,
        status: "other".into(),
    };

    let entries = vec![
        (&LogId { term: 3, index: 1 }, &req0),
        (&LogId { term: 3, index: 2 }, &req1),
        (&LogId { term: 3, index: 3 }, &req2),
    ]
    .into_iter()
    .map(|(id, req)| Entry {
        log_id: *id,
        payload: EntryPayload::Normal(EntryNormal { data: req.clone() }),
    })
    .collect::<Vec<_>>();

    store.replicate_to_state_machine(&entries.iter().collect::<Vec<_>>()).await?;
    let sm = store.get_state_machine().await;

    assert_eq!(
        sm.last_applied_log,
        LogId { term: 3, index: 3 },
        "expected last_applied_log to be 3, got {}",
        sm.last_applied_log
    );
    let client_serial1 = sm
        .client_serial_responses
        .get("1")
        .expect("expected entry to exist in client_serial_responses for client 1");
    assert_eq!(client_serial1.0, 1, "unexpected client serial response");
    assert_eq!(
        client_serial1.1,
        Some(String::from("old")),
        "unexpected client serial response"
    );
    let client_serial2 = sm
        .client_serial_responses
        .get("2")
        .expect("expected entry to exist in client_serial_responses for client 2");
    assert_eq!(client_serial2.0, 0, "unexpected client serial response");
    assert_eq!(client_serial2.1, None, "unexpected client serial response");
    let client_status1 = sm.client_status.get("1").expect("expected entry to exist in client_status for client 1");
    let client_status2 = sm.client_status.get("2").expect("expected entry to exist in client_status for client 2");
    assert_eq!(
        client_status1, "new",
        "expected client_status to be 'new', got '{}'",
        client_status1
    );
    assert_eq!(
        client_status2, "other",
        "expected client_status to be 'other', got '{}'",
        client_status2
    );
    Ok(())
}

//////////////////////////////////////////////////////////////////////////////

fn default_store_with_logs() -> MemStore {
    let mut log = BTreeMap::new();
    log.insert(1, Entry {
        log_id: (1, 1).into(),
        payload: EntryPayload::Blank,
    });
    log.insert(2, Entry {
        log_id: (1, 2).into(),
        payload: EntryPayload::Blank,
    });
    log.insert(3, Entry {
        log_id: (1, 3).into(),
        payload: EntryPayload::Blank,
    });
    log.insert(4, Entry {
        log_id: (1, 4).into(),
        payload: EntryPayload::Blank,
    });
    log.insert(5, Entry {
        log_id: (1, 5).into(),
        payload: EntryPayload::Blank,
    });
    log.insert(6, Entry {
        log_id: (1, 6).into(),
        payload: EntryPayload::Blank,
    });
    log.insert(7, Entry {
        log_id: (1, 7).into(),
        payload: EntryPayload::Blank,
    });
    log.insert(8, Entry {
        log_id: (1, 8).into(),
        payload: EntryPayload::Blank,
    });
    log.insert(9, Entry {
        log_id: (1, 9).into(),
        payload: EntryPayload::Blank,
    });
    log.insert(10, Entry {
        log_id: (1, 10).into(),
        payload: EntryPayload::Blank,
    });
    let sm = MemStoreStateMachine::default();
    let hs = HardState {
        current_term: 1,
        voted_for: Some(NODE_ID),
    };
    MemStore::new_with_state(NODE_ID, log, sm, Some(hs), None)
}
