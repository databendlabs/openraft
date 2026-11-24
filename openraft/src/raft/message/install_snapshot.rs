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
/// A transport that serializes the request with Serde enables the `serde` feature of
/// `openraft-legacy`, which also enables `openraft/serde`:
/// ```toml
/// [dependencies]
/// openraft-legacy = { version = "0.10", features = ["serde"] }
/// ```
///
/// The moved type derives `Serialize` and `Deserialize` only with this feature: `openraft/serde`
/// derived them in 0.9 and no longer does. A transport that encodes the request without Serde
/// does not need the feature.
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
/// A transport that serializes the response with Serde enables the `serde` feature of
/// `openraft-legacy`, which also enables `openraft/serde`:
/// ```toml
/// [dependencies]
/// openraft-legacy = { version = "0.10", features = ["serde"] }
/// ```
///
/// The moved type derives `Serialize` and `Deserialize` only with this feature: `openraft/serde`
/// derived them in 0.9 and no longer does. A transport that encodes the response without Serde
/// does not need the feature.
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

/// The response to [`Raft::install_full_snapshot()`], which the leader receives as the result of
/// [`NetSnapshot::full_snapshot()`].
///
/// The target returns a response, not an error, also when it rejects the leader, so the
/// response's [`vote`](Self::vote) tells the outcome. Compared with the leader's vote in the
/// request, it is:
///
/// - equal if the target accepted the leader: the target installed the snapshot, or already had
///   every log it covers. The response does not tell which.
/// - greater if the target rejected the leader: the target did not install the snapshot.
///
/// [`Raft::install_full_snapshot()`] describes when the target returns each response, and
/// [`NetSnapshot::full_snapshot()`] what the leader records for it.
///
/// A transport forwards the response that [`Raft::install_full_snapshot()`] returned to the leader
/// unchanged. Do not build one from a vote read elsewhere, such as [`RaftMetrics::vote`], which
/// can lag behind the response: an older vote can turn a rejection into a success.
///
/// [`Raft::install_full_snapshot()`]: crate::Raft::install_full_snapshot
/// [`NetSnapshot::full_snapshot()`]: crate::network::NetSnapshot::full_snapshot
/// [`RaftMetrics::vote`]: crate::RaftMetrics::vote
#[derive(Debug, Clone)]
#[derive(PartialEq, Eq)]
#[derive(derive_more::Display)]
#[display("SnapshotResponse{{vote:{}}}", vote)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct SnapshotResponse<C: RaftTypeConfig> {
    /// The target's vote when it handled the request: the leader's vote if the target accepted
    /// it, otherwise the target's greater vote.
    ///
    /// The target may send the response later, such as after the installation, but it does not
    /// read this vote again: a vote that the target accepts meanwhile is not in it.
    pub vote: VoteOf<C>,
}

impl<C: RaftTypeConfig> SnapshotResponse<C> {
    /// Create a new snapshot response with the given vote.
    pub fn new(vote: VoteOf<C>) -> Self {
        Self { vote }
    }
}
