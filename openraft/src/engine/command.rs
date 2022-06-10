use std::ops::Range;
use std::sync::Arc;

use crate::raft::VoteRequest;
use crate::EffectiveMembership;
use crate::LogId;
use crate::MetricsChangeFlags;
use crate::NodeId;
use crate::ServerState;
use crate::Vote;

/// Commands to send to `RaftRuntime` to execute, to update the application state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Command<NID: NodeId> {
    // Update server state, e.g., Leader, Follower etc.
    // TODO: consider remove this variant. A runtime does not need to know about this. It is only meant for metrics
    //       report.
    UpdateServerState {
        server_state: ServerState,
    },

    // Append a `range` of entries in the input buffer.
    AppendInputEntries {
        range: Range<usize>,
    },

    // Commit entries that are already in the store, upto `upto`, inclusive.
    // And send applied result to the client that proposed the entry.
    LeaderCommit {
        upto: LogId<NID>,
    },

    /// Commit entries that are already in the store, upto `upto`, inclusive.
    #[allow(dead_code)]
    FollowerCommit {
        upto: LogId<NID>,
    },

    // Replicate a `range` of entries in the input buffer.
    ReplicateInputEntries {
        range: Range<usize>,
    },

    // Membership config changed, need to update replication stream etc.
    UpdateMembership {
        membership: Arc<EffectiveMembership<NID>>,
    },

    // Move the cursor pointing to an entry in the input buffer.
    MoveInputCursorBy {
        n: usize,
    },

    // Save vote to storage
    SaveVote {
        vote: Vote<NID>,
    },

    /// Send vote to all other members
    SendVote {
        vote_req: VoteRequest<NID>,
    },

    /// Install a timer to trigger an election, e.g., calling `Engine::elect()` after some `timeout` which is decided
    /// by the runtime. An already installed timer should be cleared.
    InstallElectionTimer {},

    /// Reject election by other candidate for a while.
    /// The interval is decided by the runtime.
    ///
    /// When a leader is established and has not yet timeout,
    /// A candidate should not take the leadership.
    #[allow(dead_code)]
    RejectElection {},

    #[allow(dead_code)]
    PurgeLog {
        upto: LogId<NID>,
    },

    //
    // --- Draft unimplemented commands:
    //

    // TODO:
    #[allow(dead_code)]
    DeleteConflictLog {
        since: LogId<NID>,
    },

    // TODO:
    #[allow(dead_code)]
    BuildSnapshot {},
}

impl<NID: NodeId> Command<NID> {
    /// Update the flag of the metrics that needs to be updated when this command is executed.
    pub(crate) fn update_metrics_flags(&self, flags: &mut MetricsChangeFlags) {
        match &self {
            Command::UpdateServerState { .. } => flags.set_cluster_changed(),
            Command::AppendInputEntries { .. } => flags.set_data_changed(),
            Command::LeaderCommit { .. } => flags.set_data_changed(),
            Command::FollowerCommit { .. } => flags.set_data_changed(),
            Command::ReplicateInputEntries { .. } => {}
            Command::UpdateMembership { .. } => flags.set_cluster_changed(),
            Command::MoveInputCursorBy { .. } => {}
            Command::SaveVote { .. } => flags.set_data_changed(),
            Command::SendVote { .. } => {}
            Command::InstallElectionTimer { .. } => {}
            Command::RejectElection { .. } => {}
            Command::PurgeLog { .. } => flags.set_data_changed(),
            Command::DeleteConflictLog { .. } => flags.set_data_changed(),
            Command::BuildSnapshot { .. } => flags.set_data_changed(),
        }
    }
}
