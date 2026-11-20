//! Decode values that openraft 0.9.26 serialized.
//!
//! `fixtures/openraft-0.9.26.json` holds each value as `serde_json` and as `bincode` 1.3 with its
//! default options, written by 0.9.26 itself; `fixtures/gen-openraft-0.9.26` regenerates it. These
//! tests pin what the upgrade guide states about data and wire compatibility with 0.9:
//!
//! - The stored types and the RPC types whose layout did not change decode under both codecs, and
//!   the current release encodes them to the same bytes.
//! - `SnapshotMeta` and `SnapshotSignature` drop the 0.9 `snapshot_id` when they decode.
//! - `VoteRequest` and `ForwardToLeader` gained a field: JSON fills in its default, and `bincode`
//!   cannot decode the 0.9 bytes.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt::Debug;

use anyhow::Context;
use anyhow::Result;
use openraft::BasicNode;
use openraft::EntryPayload;
use openraft::Membership;
use openraft::RaftTypeConfig;
use openraft::Vote;
use openraft::alias::EntryOf;
use openraft::alias::LogIdOf;
use openraft::alias::SnapshotMetaOf;
use openraft::alias::SnapshotSignatureOf;
use openraft::alias::StoredMembershipOf;
use openraft::alias::VoteOf;
use openraft::errors::ForwardToLeader;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::vote::RaftLeaderIdExt;
use serde::Serialize;
use serde::de::DeserializeOwned;

openraft::declare_raft_types!(
    /// The type config of the 0.9.26 writer: `D = String`, `R = String`, `NodeId = u64` and
    /// `Node = BasicNode`.
    pub TypeConfig:
        D = String,
        R = String,
);

const FIXTURE: &str = include_str!("fixtures/openraft-0.9.26.json");

/// One value from the fixture, in both encodings.
struct Encoded {
    json: String,
    bincode: Vec<u8>,
}

fn fixture(name: &str) -> Result<Encoded> {
    let fixture: serde_json::Value = serde_json::from_str(FIXTURE)?;
    assert_eq!("openraft 0.9.26", fixture["writer"]);

    let values = fixture["values"].as_array().context("`values` is an array")?;
    let value = values.iter().find(|v| v["name"] == name).with_context(|| format!("no fixture `{}`", name))?;

    let json = value["json"].as_str().context("`json` is a string")?.to_string();
    let hex = value["bincode"].as_str().context("`bincode` is a string")?;
    let bincode = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Encoded { json, bincode })
}

/// Assert that both 0.9 encodings of `name` decode to `want`, and that `want` encodes to the
/// same bytes as 0.9 wrote.
///
/// The values compare by their `Debug` output, because some RPC types do not implement
/// `PartialEq`.
fn assert_unchanged<T>(name: &str, want: &T) -> Result<()>
where T: Serialize + DeserializeOwned + Debug {
    let encoded = fixture(name)?;

    let from_json: T = serde_json::from_str(&encoded.json)?;
    assert_eq!(
        format!("{:?}", want),
        format!("{:?}", from_json),
        "{}: decode json",
        name
    );
    assert_eq!(encoded.json, serde_json::to_string(want)?, "{}: encode json", name);

    let from_bincode: T = bincode::deserialize(&encoded.bincode)?;
    assert_eq!(
        format!("{:?}", want),
        format!("{:?}", from_bincode),
        "{}: decode bincode",
        name
    );
    assert_eq!(encoded.bincode, bincode::serialize(want)?, "{}: encode bincode", name);

    Ok(())
}

fn log_id(term: u64, node_id: u64, index: u64) -> LogIdOf<TypeConfig> {
    LogIdOf::<TypeConfig>::new(
        <TypeConfig as RaftTypeConfig>::LeaderId::new_committed(term, node_id),
        index,
    )
}

fn vote() -> VoteOf<TypeConfig> {
    Vote::new(3, 2)
}

fn committed_vote(term: u64, node_id: u64) -> VoteOf<TypeConfig> {
    Vote::new_committed(term, node_id)
}

/// Voters `{1,2,3}` in a joint config with `{3,4}`; node 5 is a learner.
fn membership() -> Membership<u64, BasicNode> {
    let nodes = (1..=5).map(|id| (id, BasicNode::new(format!("127.0.0.1:{}", 21000 + id))));
    let configs = vec![BTreeSet::from([1, 2, 3]), BTreeSet::from([3, 4])];
    Membership::new(configs, BTreeMap::from_iter(nodes)).unwrap()
}

fn stored_membership() -> StoredMembershipOf<TypeConfig> {
    StoredMembershipOf::<TypeConfig>::new(Some(log_id(2, 1, 5)), membership())
}

fn entries() -> Vec<EntryOf<TypeConfig>> {
    vec![
        EntryOf::<TypeConfig> {
            log_id: log_id(2, 1, 3),
            payload: EntryPayload::Blank,
        },
        EntryOf::<TypeConfig> {
            log_id: log_id(2, 1, 4),
            payload: EntryPayload::Normal("foo".to_string()),
        },
        EntryOf::<TypeConfig> {
            log_id: log_id(2, 1, 5),
            payload: EntryPayload::Membership(membership()),
        },
    ]
}

/// The types a log store and a state machine persist.
#[test]
fn test_stored_types_from_0_9() -> Result<()> {
    let [blank, normal, member] = entries().try_into().unwrap();

    assert_unchanged("vote", &vote())?;
    assert_unchanged("committed_vote", &committed_vote(3, 2))?;
    assert_unchanged("log_id", &log_id(2, 1, 5))?;
    assert_unchanged("entry_blank", &blank)?;
    assert_unchanged("entry_normal", &normal)?;
    assert_unchanged("entry_membership", &member)?;
    assert_unchanged("stored_membership", &stored_membership())?;

    Ok(())
}

/// `SnapshotMeta` and `SnapshotSignature` written by 0.9 load without their `snapshot_id`.
#[test]
fn test_snapshot_types_from_0_9() -> Result<()> {
    let want = SnapshotMetaOf::<TypeConfig> {
        last_log_id: Some(log_id(2, 1, 5)),
        last_membership: stored_membership(),
    };

    let encoded = fixture("snapshot_meta")?;
    assert!(encoded.json.contains(r#""snapshot_id":"2-1-5-7""#));

    let from_json: SnapshotMetaOf<TypeConfig> = serde_json::from_str(&encoded.json)?;
    assert_eq!(want, from_json);

    let from_bincode: SnapshotMetaOf<TypeConfig> = bincode::deserialize(&encoded.bincode)?;
    assert_eq!(want, from_bincode);

    let want = SnapshotSignatureOf::<TypeConfig> {
        last_log_id: Some(log_id(2, 1, 5)),
        last_membership_log_id: Some(Box::new(log_id(2, 1, 5))),
    };

    let encoded = fixture("snapshot_signature")?;
    assert!(encoded.json.contains(r#""snapshot_id":"2-1-5-7""#));

    let from_json: SnapshotSignatureOf<TypeConfig> = serde_json::from_str(&encoded.json)?;
    assert_eq!(want, from_json);

    let from_bincode: SnapshotSignatureOf<TypeConfig> = bincode::deserialize(&encoded.bincode)?;
    assert_eq!(want, from_bincode);

    Ok(())
}

/// The RPC types whose layout 0.10 kept.
#[test]
fn test_unchanged_rpc_types_from_0_9() -> Result<()> {
    assert_unchanged("append_entries_request", &AppendEntriesRequest::<TypeConfig> {
        vote: committed_vote(3, 2),
        prev_log_id: Some(log_id(1, 1, 2)),
        entries: entries(),
        leader_commit: Some(log_id(1, 1, 2)),
    })?;

    assert_unchanged(
        "append_entries_response_success",
        &AppendEntriesResponse::<TypeConfig>::Success,
    )?;
    assert_unchanged(
        "append_entries_response_partial_success",
        &AppendEntriesResponse::<TypeConfig>::PartialSuccess(Some(log_id(2, 1, 4))),
    )?;
    assert_unchanged(
        "append_entries_response_conflict",
        &AppendEntriesResponse::<TypeConfig>::Conflict,
    )?;
    assert_unchanged(
        "append_entries_response_higher_vote",
        &AppendEntriesResponse::<TypeConfig>::HigherVote(Vote::new(4, 3)),
    )?;

    assert_unchanged(
        "vote_response",
        &VoteResponse::<TypeConfig>::new(vote(), Some(log_id(2, 1, 5)), true),
    )?;

    Ok(())
}

/// `VoteRequest` and `ForwardToLeader` gained a field in 0.10: JSON decodes the 0.9 value with the
/// field's default, and `bincode` cannot decode it.
#[test]
fn test_rpc_types_with_a_new_field_from_0_9() -> Result<()> {
    let encoded = fixture("vote_request")?;
    let want = VoteRequest::<TypeConfig>::new(vote(), Some(log_id(2, 1, 5)));
    let from_json: VoteRequest<TypeConfig> = serde_json::from_str(&encoded.json)?;
    assert_eq!(want, from_json);
    assert!(bincode::deserialize::<VoteRequest<TypeConfig>>(&encoded.bincode).is_err());

    let encoded = fixture("forward_to_leader")?;
    let want = ForwardToLeader::<TypeConfig>::new(1, BasicNode::new("127.0.0.1:21001"));
    let from_json: ForwardToLeader<TypeConfig> = serde_json::from_str(&encoded.json)?;
    assert_eq!(want, from_json);
    assert!(bincode::deserialize::<ForwardToLeader<TypeConfig>>(&encoded.bincode).is_err());

    Ok(())
}
