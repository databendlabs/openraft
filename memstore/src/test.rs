use super::*;

const NODE_ID: u64 = 0;

//////////////////////////////////////////////////////////////////////////////
//////////////////////////////////////////////////////////////////////////////

#[tokio::test]
async fn test_get_initial_state_default() -> Result<(), MemStoreError> {
    let store = MemStore::new(NODE_ID);
    let expected_hs = HardState{current_term: 0, voted_for: None, membership: MembershipConfig::new_initial(NODE_ID)};

    let initial = store.get_initial_state().await?;

    assert_eq!(initial.last_log_index, 0, "unexpected default value for last log index");
    assert_eq!(initial.last_log_term, 0, "unexpected default value for last log term");
    assert_eq!(initial.last_applied_log, 0, "unexpected value for last applied log");
    assert_eq!(initial.hard_state, expected_hs, "unexpected value for default hard state");
    Ok(())
}

#[tokio::test]
async fn test_get_initial_state_with_previous_state() -> Result<(), MemStoreError> {
    let mut log = BTreeMap::new();
    log.insert(1, Entry{term: 1, index: 1, payload: EntryPayload::Blank});
    let mut sm = BTreeMap::new();
    sm.insert(1, Entry{term: 1, index: 1, payload: EntryPayload::Blank});
    let hs = HardState{current_term: 1, voted_for: Some(NODE_ID), membership: MembershipConfig::new_initial(NODE_ID)};
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
async fn test_save_hard_state() -> Result<(), MemStoreError> {
    let store = MemStore::new(NODE_ID);
    let new_hs = HardState{current_term: 100, voted_for: Some(NODE_ID), membership: MembershipConfig::new_initial(NODE_ID)};

    let initial = store.get_initial_state().await?;
    store.save_hard_state(&new_hs).await?;
    let post = store.get_initial_state().await?;

    assert_ne!(initial.hard_state, post.hard_state, "hard state was expected to be different after update");
    Ok(())
}

//////////////////////////////////////////////////////////////////////////////
//////////////////////////////////////////////////////////////////////////////

#[tokio::test]
async fn test_get_log_entries_returns_emptry_vec_when_start_gt_stop() -> Result<(), MemStoreError> {
    let store = default_store_with_logs();

    let logs = store.get_log_entries(10, 1).await?;

    assert_eq!(logs.len(), 0, "expected no logs to be returned");
    Ok(())
}

#[tokio::test]
async fn test_get_log_entries_returns_expected_entries() -> Result<(), MemStoreError> {
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
async fn test_delete_logs_from_does_nothing_if_start_gt_stop() -> Result<(), MemStoreError> {
    let store = default_store_with_logs();

    store.delete_logs_from(10, Some(1)).await?;
    let logs = store.get_log_entries(1, 11).await?;

    assert_eq!(logs.len(), 10, "expected all (10) logs to be preserved");
    Ok(())
}

#[tokio::test]
async fn test_delete_logs_from_deletes_target_logs() -> Result<(), MemStoreError> {
    let store = default_store_with_logs();

    store.delete_logs_from(1, Some(11)).await?;
    let logs = store.get_log_entries(0, 100).await?;

    assert_eq!(logs.len(), 0, "expected all logs to be deleted");
    Ok(())
}

#[tokio::test]
async fn test_delete_logs_from_deletes_target_logs_no_stop() -> Result<(), MemStoreError> {
    let store = default_store_with_logs();

    store.delete_logs_from(1, None).await?;
    let logs = store.get_log_entries(0, 100).await?;

    assert_eq!(logs.len(), 0, "expected all logs to be deleted");
    Ok(())
}

#[tokio::test]
async fn test_delete_logs_from_deletes_only_target_logs() -> Result<(), MemStoreError> {
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
async fn test_append_entry_to_log() -> Result<(), MemStoreError> {
    let store = default_store_with_logs();

    store.append_entry_to_log(&Entry{term: 2, index: 10, payload: EntryPayload::Blank}).await?;
    let log = store.get_log().await;

    assert_eq!(log.len(), 10, "expected 10 entries to exist in the log");
    assert_eq!(log[&10].index, 10, "unexpected log index");
    assert_eq!(log[&10].term, 2, "unexpected log term");
    Ok(())
}

//////////////////////////////////////////////////////////////////////////////
//////////////////////////////////////////////////////////////////////////////

#[tokio::test]
async fn test_replicate_to_log() -> Result<(), MemStoreError> {
    let store = default_store_with_logs();

    store.replicate_to_log(&[Entry{term: 1, index: 11, payload: EntryPayload::Blank}]).await?;
    let log = store.get_log().await;

    assert_eq!(log.len(), 11, "expected 11 entries to exist in the log");
    assert_eq!(log[&11].index, 11, "unexpected log index");
    assert_eq!(log[&11].term, 1, "unexpected log term");
    Ok(())
}

//////////////////////////////////////////////////////////////////////////////
//////////////////////////////////////////////////////////////////////////////

#[tokio::test]
async fn test_apply_entry_to_state_machine() -> Result<(), MemStoreError> {
    let store = default_store_with_logs();

    store.apply_entry_to_state_machine(&Entry{term: 1, index: 1, payload: EntryPayload::Blank}).await?;
    let sm = store.get_state_machine().await;

    assert_eq!(sm.len(), 1, "expected one entry to exist in the state machine");
    assert_eq!(sm[&1].index, 1, "unexpected entry index");
    assert_eq!(sm[&1].term, 1, "unexpected entry term");
    Ok(())
}

//////////////////////////////////////////////////////////////////////////////
//////////////////////////////////////////////////////////////////////////////

#[tokio::test]
async fn test_replicate_to_state_machine() -> Result<(), MemStoreError> {
    let store = default_store_with_logs();

    store.replicate_to_state_machine(&[
        Entry{term: 1, index: 1, payload: EntryPayload::Blank},
        Entry{term: 1, index: 2, payload: EntryPayload::Blank},
    ]).await?;
    let sm = store.get_state_machine().await;

    assert_eq!(sm.len(), 2, "expected two entries to exist in the state machine");
    assert_eq!(sm[&1].index, 1, "unexpected entry index");
    assert_eq!(sm[&1].term, 1, "unexpected entry term");
    assert_eq!(sm[&2].index, 2, "unexpected entry index");
    assert_eq!(sm[&2].term, 1, "unexpected entry term");
    Ok(())
}

//////////////////////////////////////////////////////////////////////////////
//////////////////////////////////////////////////////////////////////////////

fn default_store_with_logs() -> MemStore {
    let mut log = BTreeMap::new();
    log.insert(1, Entry{term: 1, index: 1, payload: EntryPayload::Blank});
    log.insert(2, Entry{term: 1, index: 2, payload: EntryPayload::Blank});
    log.insert(3, Entry{term: 1, index: 3, payload: EntryPayload::Blank});
    log.insert(4, Entry{term: 1, index: 4, payload: EntryPayload::Blank});
    log.insert(5, Entry{term: 1, index: 5, payload: EntryPayload::Blank});
    log.insert(6, Entry{term: 1, index: 6, payload: EntryPayload::Blank});
    log.insert(7, Entry{term: 1, index: 7, payload: EntryPayload::Blank});
    log.insert(8, Entry{term: 1, index: 8, payload: EntryPayload::Blank});
    log.insert(9, Entry{term: 1, index: 9, payload: EntryPayload::Blank});
    log.insert(10, Entry{term: 1, index: 10, payload: EntryPayload::Blank});
    let sm = BTreeMap::new();
    let hs = HardState{current_term: 1, voted_for: Some(NODE_ID), membership: MembershipConfig::new_initial(NODE_ID)};
    MemStore::new_with_state(NODE_ID, log, sm, Some(hs.clone()), None)
}
