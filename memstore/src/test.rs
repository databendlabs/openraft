use std::collections::HashSet;

use super::*;
use async_raft::raft::EntryConfigChange;

const NODE_ID: u64 = 0;

//////////////////////////////////////////////////////////////////////////////
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
    let mut members: HashSet<NodeId> = Default::default();
    members.insert(1);
    members.insert(2);
    members.insert(3);
    log.insert(
        1,
        Entry {
            term: 1,
            index: 1,
            payload: EntryPayload::ConfigChange(EntryConfigChange {
                membership: MembershipConfig {
                    members: members.clone(),
                    members_after_consensus: None,
                },
            }),
        },
    );
    let sm = MemStoreStateMachine::default();
    let hs = HardState {
        current_term: 1,
        voted_for: Some(NODE_ID),
    };
    let store = MemStore::new_with_state(NODE_ID, log, sm, Some(hs.clone()), None);

    let initial = store.get_membership_config().await?;

    assert_eq!(&initial.members, &members, "unexpected len for members");
    assert!(initial.members_after_consensus.is_none(), "unexpected value for members_after_consensus");
    Ok(())
}

//////////////////////////////////////////////////////////////////////////////
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

    assert_eq!(initial.last_log_index, 0, "unexpected default value for last log index");
    assert_eq!(initial.last_log_term, 0, "unexpected default value for last log term");
    assert_eq!(initial.last_applied_log, 0, "unexpected value for last applied log");
    assert_eq!(initial.hard_state, expected_hs, "unexpected value for default hard state");
    assert_eq!(initial.membership, expected_membership, "unexpected value for default membership config");
    Ok(())
}

#[tokio::test]
async fn test_get_initial_state_with_previous_state() -> Result<()> {
    let mut log = BTreeMap::new();
    log.insert(
        1,
        Entry {
            term: 1,
            index: 1,
            payload: EntryPayload::Blank,
        },
    );
    let sm = MemStoreStateMachine {
        last_applied_log: 1, // Just stubbed in for testing.
        ..Default::default()
    };
    let hs = HardState {
        current_term: 1,
        voted_for: Some(NODE_ID),
    };
    let store = MemStore::new_with_state(NODE_ID, log, sm, Some(hs.clone()), None);

    let initial = store.get_initial_state().await?;

    assert_eq!(initial.last_log_index, 1, "unexpected default value for last log index");
    assert_eq!(initial.last_log_term, 1, "unexpected default value for last log term");
    assert_eq!(initial.last_applied_log, 1, "unexpected value for last applied log");
    assert_eq!(initial.hard_state, hs, "unexpected value for default hard state");
    Ok(())
}

//////////////////////////////////////////////////////////////////////////////
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
    assert_eq!(logs[0].index, 5, "unexpected value for log index");
    assert_eq!(logs[0].term, 1, "unexpected value for log term");
    assert_eq!(logs[1].index, 6, "unexpected value for log index");
    assert_eq!(logs[1].term, 1, "unexpected value for log term");
    Ok(())
}

//////////////////////////////////////////////////////////////////////////////
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
    assert_eq!(logs[0].index, 10, "unexpected log index");
    Ok(())
}

//////////////////////////////////////////////////////////////////////////////
//////////////////////////////////////////////////////////////////////////////

#[tokio::test]
async fn test_append_entry_to_log() -> Result<()> {
    let store = default_store_with_logs();

    store
        .append_entry_to_log(&Entry {
            term: 2,
            index: 10,
            payload: EntryPayload::Blank,
        })
        .await?;
    let log = store.get_log().await;

    assert_eq!(log.len(), 10, "expected 10 entries to exist in the log");
    assert_eq!(log[&10].index, 10, "unexpected log index");
    assert_eq!(log[&10].term, 2, "unexpected log term");
    Ok(())
}

//////////////////////////////////////////////////////////////////////////////
//////////////////////////////////////////////////////////////////////////////

#[tokio::test]
async fn test_replicate_to_log() -> Result<()> {
    let store = default_store_with_logs();

    store
        .replicate_to_log(&[Entry {
            term: 1,
            index: 11,
            payload: EntryPayload::Blank,
        }])
        .await?;
    let log = store.get_log().await;

    assert_eq!(log.len(), 11, "expected 11 entries to exist in the log");
    assert_eq!(log[&11].index, 11, "unexpected log index");
    assert_eq!(log[&11].term, 1, "unexpected log term");
    Ok(())
}

//////////////////////////////////////////////////////////////////////////////
//////////////////////////////////////////////////////////////////////////////

#[tokio::test]
async fn test_apply_entry_to_state_machine() -> Result<()> {
    let store = default_store_with_logs();

    store
        .apply_entry_to_state_machine(
            &1,
            &ClientRequest {
                client: "0".into(),
                serial: 0,
                status: "lit".into(),
            },
        )
        .await?;
    let sm = store.get_state_machine().await;

    assert_eq!(sm.last_applied_log, 1, "expected last_applied_log to be 1, got {}", sm.last_applied_log);
    let client_serial = sm
        .client_serial_responses
        .get("0")
        .expect("expected entry to exist in client_serial_responses");
    assert_eq!(client_serial.0, 0, "unexpected client serial response");
    assert_eq!(client_serial.1, None, "unexpected client serial response");
    let client_status = sm.client_status.get("0").expect("expected entry to exist in client_status");
    assert_eq!(client_status, "lit", "expected client_status to be 'lit', got '{}'", client_status);
    Ok(())
}

//////////////////////////////////////////////////////////////////////////////
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
    let entries = vec![(&1u64, &req0), (&2u64, &req1), (&3u64, &req2)];
    store.replicate_to_state_machine(&entries).await?;
    let sm = store.get_state_machine().await;

    assert_eq!(sm.last_applied_log, 3, "expected last_applied_log to be 3, got {}", sm.last_applied_log);
    let client_serial1 = sm
        .client_serial_responses
        .get("1")
        .expect("expected entry to exist in client_serial_responses for client 1");
    assert_eq!(client_serial1.0, 1, "unexpected client serial response");
    assert_eq!(client_serial1.1, Some(String::from("old")), "unexpected client serial response");
    let client_serial2 = sm
        .client_serial_responses
        .get("2")
        .expect("expected entry to exist in client_serial_responses for client 2");
    assert_eq!(client_serial2.0, 0, "unexpected client serial response");
    assert_eq!(client_serial2.1, None, "unexpected client serial response");
    let client_status1 = sm.client_status.get("1").expect("expected entry to exist in client_status for client 1");
    let client_status2 = sm.client_status.get("2").expect("expected entry to exist in client_status for client 2");
    assert_eq!(client_status1, "new", "expected client_status to be 'new', got '{}'", client_status1);
    assert_eq!(client_status2, "other", "expected client_status to be 'other', got '{}'", client_status2);
    Ok(())
}

//////////////////////////////////////////////////////////////////////////////
//////////////////////////////////////////////////////////////////////////////

fn default_store_with_logs() -> MemStore {
    let mut log = BTreeMap::new();
    log.insert(
        1,
        Entry {
            term: 1,
            index: 1,
            payload: EntryPayload::Blank,
        },
    );
    log.insert(
        2,
        Entry {
            term: 1,
            index: 2,
            payload: EntryPayload::Blank,
        },
    );
    log.insert(
        3,
        Entry {
            term: 1,
            index: 3,
            payload: EntryPayload::Blank,
        },
    );
    log.insert(
        4,
        Entry {
            term: 1,
            index: 4,
            payload: EntryPayload::Blank,
        },
    );
    log.insert(
        5,
        Entry {
            term: 1,
            index: 5,
            payload: EntryPayload::Blank,
        },
    );
    log.insert(
        6,
        Entry {
            term: 1,
            index: 6,
            payload: EntryPayload::Blank,
        },
    );
    log.insert(
        7,
        Entry {
            term: 1,
            index: 7,
            payload: EntryPayload::Blank,
        },
    );
    log.insert(
        8,
        Entry {
            term: 1,
            index: 8,
            payload: EntryPayload::Blank,
        },
    );
    log.insert(
        9,
        Entry {
            term: 1,
            index: 9,
            payload: EntryPayload::Blank,
        },
    );
    log.insert(
        10,
        Entry {
            term: 1,
            index: 10,
            payload: EntryPayload::Blank,
        },
    );
    let sm = MemStoreStateMachine::default();
    let hs = HardState {
        current_term: 1,
        voted_for: Some(NODE_ID),
    };
    MemStore::new_with_state(NODE_ID, log, sm, Some(hs), None)
}
