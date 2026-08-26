use display_more::DisplayOptionExt;
use openraft_macros::since;

use crate::LogId;
use crate::vote::RaftCommittedLeaderId;

/// The leader has not yet committed a log entry proposed in its own term.
///
/// Raft's single-server membership change rule requires the leader to commit an entry of its
/// current term before it appends a configuration entry. Openraft proposes that entry when the
/// leader is established and records it as the leader's blank(noop) log id. Until a quorum stores
/// it, a configuration entry from an earlier term can still be committed, and two configurations
/// whose quorums do not intersect could both become committed.
///
/// A valid leader lease does not imply this condition. The lease only proves that a quorum still
/// sees this leader; it does not prove that a quorum stored the leader's blank log.
///
/// [`RaftState::cluster_committed()`] is the quorum-granted commit this error compares against.
///
/// [`RaftState::cluster_committed()`]: crate::RaftState::cluster_committed
#[since(version = "0.10.0")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("leader has not committed its own log yet: cluster committed: {}, leader log id: {leader_log_id}", committed.display())]
pub struct UncommittedLeaderLog<CLID>
where CLID: RaftCommittedLeaderId
{
    /// The greatest log id a quorum granted, i.e. the cluster-wide committed log id.
    pub committed: Option<LogId<CLID>>,

    /// The blank log id the current leader proposed when it was established.
    pub leader_log_id: LogId<CLID>,
}
