#![allow(clippy::uninlined_format_args)]

use std::collections::BTreeSet;

use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::ClientWriteResponse;
use openraft::raft::InstallSnapshotRequest;
use openraft::raft::InstallSnapshotResponse;
use openraft::raft::SnapshotResponse;
use openraft::raft::StreamAppendError;
use openraft::raft::TransferLeaderRequest;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::raft::WriteResponse;
use raft_kv_memstore_rkyv::raft::Entry;
use raft_kv_memstore_rkyv::raft::Response;
use raft_kv_memstore_rkyv::raft::SetRequest;
use raft_kv_memstore_rkyv::typ::LogId;
use raft_kv_memstore_rkyv::typ::Membership;
use raft_kv_memstore_rkyv::typ::SnapshotMeta;
use raft_kv_memstore_rkyv::typ::StoredMembership;
use raft_kv_memstore_rkyv::typ::Vote;
use raft_kv_memstore_rkyv::TypeConfig;

fn log_id(term: u64, index: u64) -> LogId {
    LogId::new_term_index(term, index)
}

fn membership() -> Membership {
    Membership::new_with_defaults(vec![BTreeSet::from([1, 2])], [3])
}

fn stored_membership() -> StoredMembership {
    StoredMembership::new(Some(log_id(4, 10)), membership())
}

fn snapshot_meta() -> SnapshotMeta {
    SnapshotMeta {
        last_log_id: Some(log_id(5, 20)),
        last_membership: stored_membership(),
        snapshot_id: "snapshot-5-20".to_string(),
    }
}

fn roundtrip<T>(value: &T) -> anyhow::Result<T>
where
    T: rkyv::Archive,
    T: for<'any> rkyv::Serialize<
        rkyv::api::high::HighSerializer<
            rkyv::util::AlignedVec,
            rkyv::ser::allocator::ArenaHandle<'any>,
            rkyv::rancor::Error,
        >,
    >,
    T::Archived: for<'arch> rkyv::bytecheck::CheckBytes<rkyv::api::high::HighValidator<'arch, rkyv::rancor::Error>>
        + rkyv::Deserialize<T, rkyv::api::high::HighDeserializer<rkyv::rancor::Error>>,
{
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(value)?;
    Ok(rkyv::from_bytes::<T, rkyv::rancor::Error>(&bytes)?)
}

#[test]
fn test_rkyv_roundtrip_vote_messages() -> anyhow::Result<()> {
    let req = VoteRequest::<TypeConfig>::new(Vote::new(2, 1), Some(log_id(2, 9)));
    let req2 = roundtrip(&req)?;
    assert_eq!(req, req2);

    let resp = VoteResponse::<TypeConfig>::new(Vote::new_committed(2, 1), Some(log_id(2, 10)), true);
    let resp2 = roundtrip(&resp)?;
    assert_eq!(resp, resp2);

    Ok(())
}

#[test]
fn test_rkyv_roundtrip_append_entries_messages() -> anyhow::Result<()> {
    let req = AppendEntriesRequest::<TypeConfig> {
        vote: Vote::new_committed(3, 1),
        prev_log_id: Some(log_id(3, 10)),
        entries: vec![Entry {
            log_id: log_id(3, 11),
            app_data: Some(SetRequest {
                key: "k".to_string(),
                value: "v".to_string(),
            }),
            membership: None,
        }],
        leader_commit: Some(log_id(3, 11)),
    };
    let req2 = roundtrip(&req)?;
    assert_eq!(req.vote, req2.vote);
    assert_eq!(req.prev_log_id, req2.prev_log_id);
    assert_eq!(req.entries, req2.entries);
    assert_eq!(req.leader_commit, req2.leader_commit);

    for resp in [
        AppendEntriesResponse::<TypeConfig>::Success,
        AppendEntriesResponse::<TypeConfig>::PartialSuccess(Some(log_id(3, 11))),
        AppendEntriesResponse::<TypeConfig>::Conflict,
        AppendEntriesResponse::<TypeConfig>::HigherVote(Vote::new_committed(4, 2)),
    ] {
        let resp2 = roundtrip(&resp)?;
        assert_eq!(resp, resp2);
    }

    Ok(())
}

#[test]
fn test_rkyv_roundtrip_snapshot_messages() -> anyhow::Result<()> {
    let meta = snapshot_meta();
    let meta2 = roundtrip(&meta)?;
    assert_eq!(meta, meta2);

    let req = InstallSnapshotRequest::<TypeConfig> {
        vote: Vote::new_committed(5, 1),
        meta: meta.clone(),
        offset: 0,
        data: vec![1, 2, 3, 4],
        done: true,
    };
    let req2 = roundtrip(&req)?;
    assert_eq!(req, req2);

    let install_resp = InstallSnapshotResponse::<TypeConfig> {
        vote: Vote::new_committed(5, 2),
    };
    let install_resp2 = roundtrip(&install_resp)?;
    assert_eq!(install_resp, install_resp2);

    let snap_resp = SnapshotResponse::<TypeConfig>::new(Vote::new_committed(5, 3));
    let snap_resp2 = roundtrip(&snap_resp)?;
    assert_eq!(snap_resp, snap_resp2);

    Ok(())
}

#[test]
fn test_rkyv_roundtrip_transfer_leader_and_write_responses() -> anyhow::Result<()> {
    let transfer_req = TransferLeaderRequest::<TypeConfig>::new(Vote::new_committed(6, 1), 2, Some(log_id(6, 30)));
    let transfer_req2 = roundtrip(&transfer_req)?;
    assert_eq!(transfer_req.from_leader(), transfer_req2.from_leader());
    assert_eq!(transfer_req.to_node_id(), transfer_req2.to_node_id());
    assert_eq!(transfer_req.last_log_id(), transfer_req2.last_log_id());

    let client_write = ClientWriteResponse::<TypeConfig> {
        log_id: log_id(7, 40),
        data: Response {
            value: Some("ok".to_string()),
        },
        membership: Some(membership()),
    };
    let client_write2 = roundtrip(&client_write)?;
    assert_eq!(client_write.log_id(), client_write2.log_id());
    assert_eq!(client_write.response(), client_write2.response());
    assert_eq!(client_write.membership(), client_write2.membership());

    let write_resp = WriteResponse::<TypeConfig> {
        log_id: log_id(7, 41),
        response: Response { value: None },
    };
    let write_resp2 = roundtrip(&write_resp)?;
    assert_eq!(write_resp.log_id, write_resp2.log_id);
    assert_eq!(write_resp.response, write_resp2.response);

    Ok(())
}

#[test]
fn test_rkyv_roundtrip_stream_append_error() -> anyhow::Result<()> {
    for err in [
        StreamAppendError::<TypeConfig>::Conflict(log_id(8, 50)),
        StreamAppendError::<TypeConfig>::HigherVote(Vote::new_committed(8, 3)),
    ] {
        let err2 = roundtrip(&err)?;
        assert_eq!(err, err2);
    }

    Ok(())
}
