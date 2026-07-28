use std::marker::PhantomData;

use crate::RaftTypeConfig;
use crate::type_config::alias::VoteOf;

/// **REMOVED**: Use `openraft_legacy::network_v1::InstallSnapshotRequest` instead.
///
/// The v1 chunk-based `InstallSnapshotRequest` has been moved to the `openraft-legacy` crate.
///
/// # Migration
///
/// Add to `Cargo.toml`:
/// ```toml
/// [dependencies]
/// openraft-legacy = "0.10"
/// ```
///
/// Update imports:
/// ```ignore
/// use openraft_legacy::network_v1::InstallSnapshotRequest;
/// ```
#[deprecated(
    since = "0.10.0",
    note = "InstallSnapshotRequest has been moved to the `openraft-legacy` crate. \
            Add `openraft-legacy` to your dependencies and use \
            `openraft_legacy::network_v1::InstallSnapshotRequest` instead."
)]
pub struct InstallSnapshotRequest<C>
where C: RaftTypeConfig
{
    _p: PhantomData<C>,
}

/// **REMOVED**: Use `openraft_legacy::network_v1::InstallSnapshotResponse` instead.
///
/// The v1 chunk-based `InstallSnapshotResponse` has been moved to the `openraft-legacy` crate.
///
/// # Migration
///
/// Add to `Cargo.toml`:
/// ```toml
/// [dependencies]
/// openraft-legacy = "0.10"
/// ```
///
/// Update imports:
/// ```ignore
/// use openraft_legacy::network_v1::InstallSnapshotResponse;
/// ```
#[deprecated(
    since = "0.10.0",
    note = "InstallSnapshotResponse has been moved to the `openraft-legacy` crate. \
            Add `openraft-legacy` to your dependencies and use \
            `openraft_legacy::network_v1::InstallSnapshotResponse` instead."
)]
pub struct InstallSnapshotResponse<C>
where C: RaftTypeConfig
{
    _p: PhantomData<C>,
}

/// The response to `Raft::install_full_snapshot` API.
#[derive(Debug, Clone)]
#[derive(PartialEq, Eq)]
#[derive(derive_more::Display)]
#[display("SnapshotResponse{{vote:{}}}", vote)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct SnapshotResponse<C: RaftTypeConfig> {
    /// The responder's current vote.
    pub vote: VoteOf<C>,
}

impl<C: RaftTypeConfig> SnapshotResponse<C> {
    /// Create a new snapshot response with the given vote.
    pub fn new(vote: VoteOf<C>) -> Self {
        Self { vote }
    }
}
