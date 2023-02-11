use std::ops::Range;
use std::sync::Arc;

use crate::error::AppendEntriesError;
use crate::error::InstallSnapshotError;
use crate::error::VoteError;
use crate::progress::entry::ProgressEntry;
use crate::progress::Inflight;
use crate::raft::AppendEntriesResponse;
use crate::raft::AppendEntriesTx;
use crate::raft::InstallSnapshotResponse;
use crate::raft::InstallSnapshotTx;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::raft::VoteTx;
use crate::EffectiveMembership;
use crate::LogId;
use crate::MetricsChangeFlags;
use crate::Node;
use crate::NodeId;
use crate::SnapshotMeta;
use crate::Vote;

/// Commands to send to `RaftRuntime` to execute, to update the application state.
#[derive(derivative::Derivative)]
#[derivative(PartialEq)]
#[derive(Debug)]
pub(crate) enum Command<NID, N>
where
    N: Node,
    NID: NodeId,
{
    /// Becomes a leader, i.e., its `vote` is granted by a quorum.
    /// The runtime initializes leader data when receives this command.
    BecomeLeader,

    /// No longer a leader. Clean up leader's data.
    QuitLeader,

    /// Append a `range` of entries in the input buffer.
    AppendInputEntries { range: Range<usize> },

    /// Append a blank log.
    ///
    /// One of the usage is when a leader is established, a blank log is written to commit the
    /// state.
    AppendBlankLog { log_id: LogId<NID> },

    /// Replicate the committed log id to other nodes
    ReplicateCommitted { committed: Option<LogId<NID>> },

    /// Commit entries that are already in the store, upto `upto`, inclusive.
    /// And send applied result to the client that proposed the entry.
    LeaderCommit {
        // TODO: pass the log id list?
        // TODO: merge LeaderCommit and FollowerCommit
        already_committed: Option<LogId<NID>>,
        upto: LogId<NID>,
    },

    /// Commit entries that are already in the store, upto `upto`, inclusive.
    FollowerCommit {
        already_committed: Option<LogId<NID>>,
        upto: LogId<NID>,
    },

    /// Replicate log entries or snapshot to a target.
    Replicate { target: NID, req: Inflight<NID> },

    // /// Replicate a snapshot to a target.
    // ReplicateSnapshot {
    //     target: NID,
    //     snapshot_last_log_id: Option<LogId<NID>>,
    // },
    /// Membership config changed, need to update replication streams.
    UpdateMembership {
        // TODO: not used yet.
        membership: Arc<EffectiveMembership<NID, N>>,
    },

    /// Membership config changed, need to update replication streams.
    /// The Runtime has to close all old replications and start new ones.
    /// Because a replication stream should only report state for one membership config.
    /// When membership config changes, the membership log id stored in ReplicationCore has to be
    /// updated.
    RebuildReplicationStreams {
        /// Targets to replicate to.
        targets: Vec<(NID, ProgressEntry<NID>)>,
    },

    // TODO(3): it also update the progress of a leader.
    //          Add doc:
    //          `target` can also be the leader id.
    /// As the state of replication to `target` is updated, the metrics should be updated.
    UpdateProgressMetrics { target: NID, matching: LogId<NID> },

    /// Move the cursor pointing to an entry in the input buffer.
    MoveInputCursorBy { n: usize },

    /// Save vote to storage
    SaveVote { vote: Vote<NID> },

    /// Send vote to all other members
    SendVote { vote_req: VoteRequest<NID> },

    /// Install a timer to trigger an election, e.g., calling `Engine::elect()` after some
    /// `timeout` which is decided by the runtime. An already installed timer should be
    /// cleared.
    InstallElectionTimer {
        /// When a candidate fails to elect, it falls back to follower.
        /// If many enough greater last-log-ids are seen, then this node can never become a leader.
        /// Thus give it a longer sleep time before next election.
        can_be_leader: bool,
    },

    /// Purge log from the beginning to `upto`, inclusive.
    PurgeLog { upto: LogId<NID> },

    /// Delete logs that conflict with the leader from a follower/learner since log id `since`,
    /// inclusive.
    DeleteConflictLog { since: LogId<NID> },

    /// Install a snapshot data file: e.g., replace state machine with snapshot, save snapshot
    /// data.
    InstallSnapshot { snapshot_meta: SnapshotMeta<NID, N> },

    /// A received snapshot does not need to be installed, just drop buffered snapshot data.
    CancelSnapshot { snapshot_meta: SnapshotMeta<NID, N> },

    // ---
    // --- Response commands
    // ---
    /// Send vote result `res` to its caller via `tx`
    SendVoteResult {
        res: Result<VoteResponse<NID>, VoteError<NID>>,
        #[derivative(PartialEq = "ignore")]
        tx: VoteTx<NID>,
    },

    /// Send append-entries result `res` to its caller via `tx`
    SendAppendEntriesResult {
        res: Result<AppendEntriesResponse<NID>, AppendEntriesError<NID>>,
        #[derivative(PartialEq = "ignore")]
        tx: AppendEntriesTx<NID>,
    },

    // TODO: use it
    #[allow(dead_code)]
    /// Send install-snapshot result `res` to its caller via `tx`
    SendInstallSnapshotResult {
        res: Result<InstallSnapshotResponse<NID>, InstallSnapshotError<NID>>,
        #[derivative(PartialEq = "ignore")]
        tx: InstallSnapshotTx<NID>,
    },

    //
    // --- Draft unimplemented commands:

    // TODO:
    #[allow(dead_code)]
    BuildSnapshot {},
}

impl<NID, N> Eq for Command<NID, N>
where
    N: Node,
    NID: NodeId,
{
}

impl<NID, N> Command<NID, N>
where
    N: Node,
    NID: NodeId,
{
    /// Update the flag of the metrics that needs to be updated when this command is executed.
    pub(crate) fn update_metrics_flags(&self, flags: &mut MetricsChangeFlags) {
        match &self {
            Command::BecomeLeader { .. } => flags.set_cluster_changed(),
            Command::QuitLeader => flags.set_cluster_changed(),
            Command::AppendInputEntries { .. } => flags.set_data_changed(),
            Command::AppendBlankLog { .. } => flags.set_data_changed(),
            Command::ReplicateCommitted { .. } => {}
            Command::LeaderCommit { .. } => flags.set_data_changed(),
            Command::FollowerCommit { .. } => flags.set_data_changed(),
            Command::Replicate { .. } => {}
            Command::UpdateMembership { .. } => flags.set_cluster_changed(),
            Command::RebuildReplicationStreams { .. } => flags.set_replication_changed(),
            Command::UpdateProgressMetrics { .. } => flags.set_replication_changed(),
            Command::MoveInputCursorBy { .. } => {}
            Command::SaveVote { .. } => flags.set_data_changed(),
            Command::SendVote { .. } => {}
            Command::InstallElectionTimer { .. } => {}
            Command::PurgeLog { .. } => flags.set_data_changed(),
            Command::DeleteConflictLog { .. } => flags.set_data_changed(),
            Command::InstallSnapshot { .. } => flags.set_data_changed(),
            Command::CancelSnapshot { .. } => {}
            Command::BuildSnapshot { .. } => flags.set_data_changed(),
            Command::SendVoteResult { .. } => {}
            Command::SendAppendEntriesResult { .. } => {}
            Command::SendInstallSnapshotResult { .. } => {}
        }
    }
}
