use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::Mutex;

use openraft::EntryPayload;
use openraft::storage::IOFlushed;
use openraft::storage::RaftLogReader;
use openraft::storage::RaftLogStorage;
use openraft::vote::RaftLeaderId;

use super::EntryHistory;
use crate::storage_model::StorageModel;
use crate::store::LogStore;
use crate::typ::*;

fn entry() -> Entry {
    Entry {
        log_id: LogId::new(openraft::impls::leader_id_adv::LeaderId::new(1, 1), 7),
        payload: EntryPayload::Normal(Request {
            client_id: "client".to_string(),
            serial: 1,
            key: "key".to_string(),
            value: "before".to_string(),
        }),
    }
}

#[test]
fn replication_and_new_leader_ids_preserve_identity() {
    let mut history = EntryHistory::default();
    let first = entry();
    history.record(1, &first);
    history.record(2, &first);
    history.record(1, &first);

    let mut replacement = first.clone();
    replacement.log_id = LogId::new(openraft::impls::leader_id_adv::LeaderId::new(2, 1), 7);
    replacement.payload = EntryPayload::Blank;
    history.record(1, &replacement);
    replacement.log_id = LogId::new(openraft::impls::leader_id_adv::LeaderId::new(1, 2), 7);
    history.record(2, &replacement);

    let violations = history.drain_violations();
    assert_eq!(violations, Vec::<String>::new());
}

#[test]
fn entry_lost_before_reaching_another_node_may_be_reissued() {
    let mut history = EntryHistory::default();
    let lost = entry();
    history.record(1, &lost);
    let mut reissued = lost.clone();
    reissued.payload = EntryPayload::Blank;
    history.record(1, &reissued);
    history.record(2, &reissued);
    let violations = history.drain_violations();
    assert_eq!(violations, Vec::<String>::new());

    history.record(3, &lost);
    let expected = format!(
        "LogIdReused: T1-N1.7 held by nodes {{1, 2}}: blank; now on n3: {:?}",
        lost.payload
    );
    let violations = history.drain_violations();
    assert_eq!(violations, vec![expected]);
}

#[test]
fn every_request_field_is_part_of_identity() {
    let first = entry();
    let EntryPayload::Normal(request) = &first.payload else {
        unreachable!();
    };
    let mut variants = vec![request.clone(); 4];
    variants[0].client_id = "other-client".to_string();
    variants[1].serial = 2;
    variants[2].key = "other-key".to_string();
    variants[3].value = "after".to_string();

    for request in variants {
        let mut history = EntryHistory::default();
        history.record(1, &first);
        let second = Entry {
            log_id: first.log_id,
            payload: EntryPayload::Normal(request),
        };
        history.record(2, &second);
        let expected = format!(
            "LogIdReused: T1-N1.7 held by nodes {{1}}: {:?}; now on n2: {:?}",
            first.payload, second.payload
        );
        let violations = history.drain_violations();
        assert_eq!(violations, vec![expected]);
        let drained = history.drain_violations();
        assert_eq!(drained, Vec::<String>::new());
    }
}

#[test]
fn blank_and_membership_payloads_are_checked() {
    let first_voters = BTreeSet::from([1, 2, 3]);
    let other_voters = BTreeSet::from([1, 2, 4]);
    let first_membership = majority_membership(vec![first_voters], vec![]);
    let other_membership = majority_membership(vec![other_voters], vec![]);
    let payloads = vec![
        EntryPayload::Blank,
        entry().payload,
        EntryPayload::Membership(first_membership),
        EntryPayload::Membership(other_membership),
    ];

    for first_payload in &payloads {
        for second_payload in &payloads {
            let mut history = EntryHistory::default();
            let mut observed = entry();
            observed.payload = first_payload.clone();
            history.record(1, &observed);
            observed.payload = second_payload.clone();
            history.record(2, &observed);
            let expected = if first_payload == second_payload {
                vec![]
            } else {
                vec![format!(
                    "LogIdReused: T1-N1.7 held by nodes {{1}}: {first_payload:?}; now on n2: {second_payload:?}"
                )]
            };
            let violations = history.drain_violations();
            assert_eq!(violations, expected);
        }
    }
}

#[tokio::test]
async fn append_history_survives_power_loss_truncation_and_purge() {
    let history = Arc::new(Mutex::new(EntryHistory::default()));
    let model = StorageModel {
        async_flush: true,
        flush_delay_ms: (1000, 1000),
        ..StorageModel::baseline()
    };
    let store = LogStore::new(model, 0);
    let mut leader = Arc::new(store.with_history(1, history.clone()));
    let store = LogStore::new(StorageModel::baseline(), 1);
    let mut follower = Arc::new(store.with_history(2, history.clone()));
    let first = entry();
    leader.append([first.clone()], IOFlushed::noop()).await.unwrap();
    follower.append([first.clone()], IOFlushed::noop()).await.unwrap();

    leader.crash(true);
    let recovered = leader.try_get_log_entries(..).await.unwrap();
    assert_eq!(recovered, vec![]);
    follower.truncate_after(None).await.unwrap();
    follower.append([first.clone()], IOFlushed::noop()).await.unwrap();
    follower.purge(first.log_id).await.unwrap();
    let existing = history.lock().unwrap().drain_violations();
    assert_eq!(existing, Vec::<String>::new());

    let mut second = first.clone();
    second.payload = EntryPayload::Blank;
    leader.append([second], IOFlushed::noop()).await.unwrap();
    let violations = history.lock().unwrap().drain_violations();
    assert_eq!(violations, vec![
        "LogIdReused: T1-N1.7 held by nodes {1, 2}: normal:Request { client_id: \"client\", serial: 1, key: \"key\", value: \"before\" }; now on n1: blank".to_string(),
    ]);
}
