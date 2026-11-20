//! Serialize openraft 0.9.26 values with the codecs the upgrade guide covers:
//! `serde_json`, a named-field format, and `bincode` 1.3 with its default
//! options, a positional one.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt::Write;
use std::path::PathBuf;

use openraft::error::ForwardToLeader;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::BasicNode;
use openraft::CommittedLeaderId;
use openraft::Entry;
use openraft::EntryPayload;
use openraft::LogId;
use openraft::Membership;
use openraft::SnapshotMeta;
use openraft::StoredMembership;
use openraft::Vote;
use serde::Serialize;

openraft::declare_raft_types!(
    pub TypeConfig:
        D = String,
        R = String,
        NodeId = u64,
        Node = BasicNode,
        Entry = Entry<TypeConfig>,
        SnapshotData = std::io::Cursor<Vec<u8>>,
        AsyncRuntime = openraft::TokioRuntime
);

fn log_id(term: u64, node_id: u64, index: u64) -> LogId<u64> {
    LogId::new(CommittedLeaderId::new(term, node_id), index)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut s, b| {
        write!(s, "{b:02x}").unwrap();
        s
    })
}

fn record<T>(values: &mut Vec<serde_json::Value>, name: &str, value: &T)
where T: Serialize {
    values.push(serde_json::json!({
        "name": name,
        "json": serde_json::to_string(value).unwrap(),
        "bincode": hex(&bincode::serialize(value).unwrap()),
    }));
}

fn main() {
    let nodes = (1..=4).map(|id| (id, BasicNode::new(format!("127.0.0.1:{}", 21000 + id))));
    // Voters 1, 2 and 3 in a joint config with 3 and 4; node 5 is a learner.
    let mut nodes = BTreeMap::from_iter(nodes);
    nodes.insert(5, BasicNode::new("127.0.0.1:21005"));
    let membership = Membership::new(vec![BTreeSet::from([1, 2, 3]), BTreeSet::from([3, 4])], nodes);

    let vote = Vote::new(3, 2);
    let committed_vote = Vote::new_committed(3, 2);
    let last = log_id(2, 1, 5);

    let blank = Entry::<TypeConfig> {
        log_id: log_id(2, 1, 3),
        payload: EntryPayload::Blank,
    };
    let normal = Entry::<TypeConfig> {
        log_id: log_id(2, 1, 4),
        payload: EntryPayload::Normal("foo".to_string()),
    };
    let member = Entry::<TypeConfig> {
        log_id: last,
        payload: EntryPayload::Membership(membership.clone()),
    };
    let stored_membership = StoredMembership::new(Some(last), membership);

    let mut values = vec![];
    record(&mut values, "vote", &vote);
    record(&mut values, "committed_vote", &committed_vote);
    record(&mut values, "log_id", &last);
    record(&mut values, "entry_blank", &blank);
    record(&mut values, "entry_normal", &normal);
    record(&mut values, "entry_membership", &member);
    record(&mut values, "stored_membership", &stored_membership);
    let snapshot_meta = SnapshotMeta {
        last_log_id: Some(last),
        last_membership: stored_membership,
        snapshot_id: "2-1-5-7".to_string(),
    };
    record(&mut values, "snapshot_signature", &snapshot_meta.signature());
    record(&mut values, "snapshot_meta", &snapshot_meta);
    let append = AppendEntriesRequest::<TypeConfig> {
        vote: committed_vote,
        prev_log_id: Some(log_id(1, 1, 2)),
        entries: vec![blank, normal, member],
        leader_commit: Some(log_id(1, 1, 2)),
    };
    record(&mut values, "append_entries_request", &append);
    record(
        &mut values,
        "append_entries_response_success",
        &AppendEntriesResponse::<u64>::Success,
    );
    record(
        &mut values,
        "append_entries_response_partial_success",
        &AppendEntriesResponse::<u64>::PartialSuccess(Some(log_id(2, 1, 4))),
    );
    record(
        &mut values,
        "append_entries_response_conflict",
        &AppendEntriesResponse::<u64>::Conflict,
    );
    record(
        &mut values,
        "append_entries_response_higher_vote",
        &AppendEntriesResponse::<u64>::HigherVote(Vote::new(4, 3)),
    );
    record(&mut values, "vote_request", &VoteRequest::new(vote, Some(last)));
    record(&mut values, "vote_response", &VoteResponse::new(vote, Some(last), true));
    record(
        &mut values,
        "forward_to_leader",
        &ForwardToLeader::<u64, BasicNode>::new(1, BasicNode::new("127.0.0.1:21001")),
    );

    let fixture = serde_json::json!({
        "writer": "openraft 0.9.26",
        "type_config": "D = String, R = String, NodeId = u64, Node = BasicNode",
        "codecs": {
            "json": "serde_json::to_string",
            "bincode": "bincode 1.3.3 bincode::serialize: default options, fixed-width little-endian integers; hex-encoded",
        },
        "values": values,
    });

    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../openraft-0.9.26.json");
    std::fs::write(out, serde_json::to_string_pretty(&fixture).unwrap() + "\n").unwrap();
}
