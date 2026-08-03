//! Positional-format compatibility with the 0.9 serialized layouts.
//!
//! A positional format such as bincode encodes a struct as a bare sequence: no field names,
//! only the count and order. The exact-JSON unit tests in `openraft` and `openraft-legacy` pin
//! that count by proxy; these tests exercise it directly, in both directions, for each type
//! that crosses a 0.9/0.10 boundary: stored `SnapshotMeta`, `SnapshotSignature` inside errors,
//! and the v1 chunked `InstallSnapshotRequest`.
//!
//! The 0.9 replicas below reuse the current inner types (`LogId`, `Vote`, `StoredMembership`),
//! whose layouts are unchanged since 0.9; the field under test is `snapshot_id`.

use anyhow::Result;
use maplit::btreeset;
use openraft::Membership;
use openraft::Vote;
use openraft::alias::LogIdOf;
use openraft::alias::SnapshotMetaOf;
use openraft::alias::SnapshotSignatureOf;
use openraft::alias::StoredMembershipOf;
use openraft::alias::VoteOf;
use openraft_legacy::network_v1::InstallSnapshotRequest;
use openraft_legacy::network_v1::SnapshotMeta as SnapshotMetaV1;
use openraft_memstore::TypeConfig;
use serde::Deserialize;
use serde::Serialize;

use crate::fixtures::log_id;

/// The 0.9 `SnapshotMeta` layout: `snapshot_id` is a third, meaningful field.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct SnapshotMeta09 {
    last_log_id: Option<LogIdOf<TypeConfig>>,
    last_membership: StoredMembershipOf<TypeConfig>,
    snapshot_id: String,
}

/// The 0.9 `SnapshotSignature` layout.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct SnapshotSignature09 {
    last_log_id: Option<LogIdOf<TypeConfig>>,
    last_membership_log_id: Option<Box<LogIdOf<TypeConfig>>>,
    snapshot_id: String,
}

/// The 0.9 `InstallSnapshotRequest` layout: five fields, the id inside `meta`.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct InstallSnapshotRequest09 {
    vote: VoteOf<TypeConfig>,
    meta: SnapshotMeta09,
    offset: u64,
    data: Vec<u8>,
    done: bool,
}

fn membership() -> StoredMembershipOf<TypeConfig> {
    StoredMembershipOf::<TypeConfig>::new(
        Some(log_id(1, 1, 1)),
        Membership::new_with_defaults(vec![btreeset! {1,2}], []),
    )
}

/// A `SnapshotMeta` written by 0.9 loads as the 0.10 type; the id is ignored.
#[test]
fn test_snapshot_meta_bincode_from_09() -> Result<()> {
    let old = SnapshotMeta09 {
        last_log_id: Some(log_id(1, 2, 3)),
        last_membership: membership(),
        snapshot_id: "1-2-3-4".to_string(),
    };

    let new: SnapshotMetaOf<TypeConfig> = bincode::deserialize(&bincode::serialize(&old)?)?;

    assert_eq!(
        SnapshotMetaOf::<TypeConfig> {
            last_log_id: Some(log_id(1, 2, 3)),
            last_membership: membership(),
        },
        new
    );
    Ok(())
}

/// A `SnapshotMeta` written by 0.10 loads as the 0.9 layout; the reserved id slot is empty.
#[test]
fn test_snapshot_meta_bincode_to_09() -> Result<()> {
    let new = SnapshotMetaOf::<TypeConfig> {
        last_log_id: Some(log_id(1, 2, 3)),
        last_membership: membership(),
    };

    let old: SnapshotMeta09 = bincode::deserialize(&bincode::serialize(&new)?)?;

    assert_eq!(
        SnapshotMeta09 {
            last_log_id: Some(log_id(1, 2, 3)),
            last_membership: membership(),
            snapshot_id: "".to_string(),
        },
        old
    );
    Ok(())
}

/// A `SnapshotSignature` from a 0.9 error loads as the 0.10 type; the id is ignored.
#[test]
fn test_snapshot_signature_bincode_from_09() -> Result<()> {
    let old = SnapshotSignature09 {
        last_log_id: Some(log_id(1, 2, 3)),
        last_membership_log_id: Some(Box::new(log_id(1, 1, 1))),
        snapshot_id: "1-2-3-4".to_string(),
    };

    let new: SnapshotSignatureOf<TypeConfig> = bincode::deserialize(&bincode::serialize(&old)?)?;

    assert_eq!(
        SnapshotSignatureOf::<TypeConfig> {
            last_log_id: Some(log_id(1, 2, 3)),
            last_membership_log_id: Some(Box::new(log_id(1, 1, 1))),
        },
        new
    );
    Ok(())
}

/// A `SnapshotSignature` from a 0.10 error loads as the 0.9 layout; the id slot is empty.
#[test]
fn test_snapshot_signature_bincode_to_09() -> Result<()> {
    let new = SnapshotSignatureOf::<TypeConfig> {
        last_log_id: Some(log_id(1, 2, 3)),
        last_membership_log_id: Some(Box::new(log_id(1, 1, 1))),
    };

    let old: SnapshotSignature09 = bincode::deserialize(&bincode::serialize(&new)?)?;

    assert_eq!(
        SnapshotSignature09 {
            last_log_id: Some(log_id(1, 2, 3)),
            last_membership_log_id: Some(Box::new(log_id(1, 1, 1))),
            snapshot_id: "".to_string(),
        },
        old
    );
    Ok(())
}

/// The v1 RPC round-trips between the 0.9 and 0.10 layouts with the transfer id preserved:
/// unlike the storage types above, the id inside the request `meta` is meaningful.
#[test]
fn test_install_snapshot_request_bincode_roundtrip_09() -> Result<()> {
    let old = InstallSnapshotRequest09 {
        vote: Vote::new_committed(2, 1),
        meta: SnapshotMeta09 {
            last_log_id: Some(log_id(1, 2, 3)),
            last_membership: membership(),
            snapshot_id: "ss-1".to_string(),
        },
        offset: 7,
        data: vec![1, 2, 3],
        done: true,
    };

    let new: InstallSnapshotRequest<TypeConfig> = bincode::deserialize(&bincode::serialize(&old)?)?;

    assert_eq!(
        InstallSnapshotRequest::<TypeConfig> {
            vote: Vote::new_committed(2, 1),
            meta: SnapshotMetaV1 {
                last_log_id: Some(log_id(1, 2, 3)),
                last_membership: membership(),
                snapshot_id: "ss-1".to_string(),
            },
            offset: 7,
            data: vec![1, 2, 3],
            done: true,
        },
        new
    );

    let back: InstallSnapshotRequest09 = bincode::deserialize(&bincode::serialize(&new)?)?;
    assert_eq!(old, back);

    Ok(())
}
