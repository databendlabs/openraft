use display_more::DisplayOptionExt;
use openraft_macros::since;

use crate::LogId;
use crate::vote::RaftCommittedLeaderId;

/// The leader has not yet committed a log entry proposed in its own term.
///
/// A single-step membership change requires the leader to commit a new log entry of its current
/// term before it appends a configuration entry. Openraft proposes that entry when the leader is
/// established and records it as the leader's blank (noop) log id. Until a quorum stores it, a
/// configuration entry from an earlier term can still be committed, and two configurations whose
/// quorums do not intersect could both become committed.
///
/// A valid leader lease does not imply this condition. The lease only proves that a quorum still
/// sees this leader; it does not prove that a quorum stored the leader's blank log.
///
/// [`RaftState::cluster_committed()`] is the quorum-granted commit this error compares against.
///
/// # Example
///
/// The cluster starts with the committed membership `[{a,b,c,d}]`, whose quorum is three voters.
/// `u` and `v` are two nodes that are not voters yet. Both configurations below add one node to
/// `[{a,b,c,d}]`, so [`UnsupportedMembershipTransition`] accepts both.
///
/// 1. Term 1: `a` is elected, and without committing any term-1 log it appends `[{a,b,c,d,u}]` at
///    index 6. The entry reaches only `a` and `u`. Then `a` crashes.
/// 2. Term 2: `b`, `c` and `d` elect `d`, because none of them holds index 6.
/// 3. `d` appends `[{a,b,c,d,v}]` at index 6. The entry reaches `c`, `d` and `v`, a quorum of
///    `[{a,b,c,d,v}]`, so this configuration **is committed**. Then `d` crashes.
/// 4. Term 3: `a` restarts and wins with `a`, `b` and `u`, a quorum of the `[{a,b,c,d,u}]` from
///    step 1. `b` grants the vote because `d` never replicated to `b`, leaving `b` at index 5.
/// 5. `a` replicates its own index 6 over `c` and `d`, destroying the committed `[{a,b,c,d,v}]`.
///
/// Quorum `{a,b,u}` and quorum `{c,d,v}` share no node, so neither election blocked the other.
/// Committing a term-1 log before step 1 is what rules this run out.
///
/// # References
///
/// - [Bug in single-server membership changes][ongaro-bug]: Diego Ongaro's report of this bug,
///   which introduced the rule this error enforces.
/// - [The Pitfalls of Raft Membership Change][pitfalls]: a walk-through of a run that loses a
///   committed configuration when the rule is not enforced.
///
/// [`RaftState::cluster_committed()`]: crate::RaftState::cluster_committed
/// [`UnsupportedMembershipTransition`]: crate::errors::UnsupportedMembershipTransition
/// [ongaro-bug]: https://gist.github.com/ongardie/a11f32b70581e20d6bcd
/// [pitfalls]: https://blog.openacid.com/distributed/raft-bug/
#[since(version = "0.10.0")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error(
    "single-step membership change is blocked: the leader must first commit a new log of its current term, \
     but its blank log {leader_log_id} is not committed; cluster committed: {}",
    committed.display()
)]
pub struct UncommittedLeaderLog<CLID>
where CLID: RaftCommittedLeaderId
{
    /// The greatest log id a quorum granted, i.e. the cluster-wide committed log id.
    pub committed: Option<LogId<CLID>>,

    /// The blank log id the current leader proposed when it was established.
    pub leader_log_id: LogId<CLID>,
}
