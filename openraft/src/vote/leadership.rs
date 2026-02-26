use std::fmt;

use crate::RaftTypeConfig;
use crate::display_ext::DisplayOptionExt;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::VoteOf;

/// Represents a node claiming leadership over the Raft cluster.
///
/// A **leader** here refers to any node asserting leadership authority — either:
/// - An **established leader** that has committed its vote and is sending AppendEntries RPCs, or
/// - A **candidate** that is still collecting votes via VoteRequest RPCs.
///
/// In both cases, the claimant is expected to hold the **greatest vote** and the **most
/// up-to-date log** (`last_log_id`) among all nodes it communicates with.  A follower
/// accepts the claim only when both conditions are satisfied:
/// - `vote >= local_vote` — the claimant's term is at least as high as what the follower has
///   already seen or promised.
/// - `last_log_id >= local_last_log_id` — the claimant's log is at least as up-to-date as the
///   follower's, guaranteeing that no committed entries are lost.
///
/// ## The two axes of leadership authority
///
/// The two fields encode orthogonal dimensions of the leader's authority:
///
/// - **`vote` represents time** — the term is a logical clock that advances whenever a new election
///   begins.  A higher term means the claim originates from a more recent epoch. Granting a vote or
///   accepting an AppendEntries with a higher term acknowledges that the old leader's authority has
///   expired and a new one is taking over.
///
/// - **`last_log_id` represents event history** — the log is the ordered record of every
///   state-machine command the cluster has agreed to execute.  A higher `last_log_id` means the
///   claimant has witnessed more of that history.  Requiring `last_log_id >= local` ensures that no
///   already-committed entry can be silently overwritten by a newly elected leader.
///
/// A valid leader must dominate on **both** axes: it must speak from a recent enough epoch
/// *and* must carry at least as much history as the follower it is addressing.
///
/// ## Why the last-log-id check is implicit for AppendEntries
///
/// When a candidate wins an election, every quorum member has already validated that the
/// candidate's log is at least as up-to-date as its own.  Therefore, by the time the elected
/// leader sends AppendEntries, the vote itself is sufficient proof that the log-completeness
/// requirement is satisfied.  Openraft exploits this: it omits the explicit `last_log_id` check
/// in AppendEntries processing, treating `last_log_id = None` as "check already done".
pub struct Leadership<C: RaftTypeConfig> {
    /// The logical clock of this leadership claim.
    ///
    /// Represents *time*: a higher vote means a more recent epoch.
    pub vote: VoteOf<C>,

    /// The last log id the leader holds.
    ///
    /// Represents *event history*: a higher value means the claimant has seen more committed
    /// entries.  `None` is a sentinel meaning **skip the last-log-id check** — the caller
    /// asserts that the check is unnecessary or has already been performed elsewhere (e.g.,
    /// during election).
    pub last_log_id: Option<LogIdOf<C>>,
}

impl<C: RaftTypeConfig> fmt::Display for Leadership<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "vote:{}, last_log_id:{}", self.vote, self.last_log_id.display())
    }
}
