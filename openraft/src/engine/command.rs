use std::fmt::Debug;

use tokio::sync::oneshot;

use crate::entry::RaftEntry;
use crate::error::Infallible;
use crate::error::InitializeError;
use crate::error::InstallSnapshotError;
use crate::progress::entry::ProgressEntry;
use crate::progress::Inflight;
use crate::raft::AppendEntriesResponse;
use crate::raft::InstallSnapshotResponse;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::LogId;
use crate::MetricsChangeFlags;
use crate::Node;
use crate::NodeId;
use crate::SnapshotMeta;
use crate::Vote;

/// Commands to send to `RaftRuntime` to execute, to update the application state.
#[derive(Debug)]
pub(crate) enum Command<NID, N, Ent>
where
    NID: NodeId,
    N: Node,
    Ent: RaftEntry<NID, N>,
{
    /// Becomes a leader, i.e., its `vote` is granted by a quorum.
    /// The runtime initializes leader data when receives this command.
    BecomeLeader,

    /// No longer a leader. Clean up leader's data.
    QuitLeader,

    /// Append one entry.
    AppendEntry { entry: Ent },

    /// Append a `range` of entries.
    AppendInputEntries { entries: Vec<Ent> },

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

    /// Save vote to storage
    SaveVote { vote: Vote<NID> },

    /// Send vote to all other members
    SendVote { vote_req: VoteRequest<NID> },

    /// Purge log from the beginning to `upto`, inclusive.
    PurgeLog { upto: LogId<NID> },

    /// Delete logs that conflict with the leader from a follower/learner since log id `since`,
    /// inclusive.
    DeleteConflictLog { since: LogId<NID> },

    /// Install a snapshot data file: e.g., replace state machine with snapshot, save snapshot
    /// data.
    InstallSnapshot { snapshot_meta: SnapshotMeta<NID, N> },

    // TODO: remove this, use InstallSnapshot instead.
    /// A received snapshot does not need to be installed, just drop buffered snapshot data.
    CancelSnapshot { snapshot_meta: SnapshotMeta<NID, N> },

    // ---
    // --- Response commands
    // ---
    /// Send result to caller
    Respond { resp: Respond<NID, N> },

    //
    // --- Draft unimplemented commands:

    // TODO:
    #[allow(dead_code)]
    BuildSnapshot {},
}

/// For unit testing
impl<NID, N, Ent> PartialEq for Command<NID, N, Ent>
where
    NID: NodeId,
    N: Node,
    Ent: RaftEntry<NID, N> + PartialEq,
{
    #[rustfmt::skip]
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Command::BecomeLeader,                                Command::BecomeLeader)                                                        => true,
            (Command::QuitLeader,                                  Command::QuitLeader)                                                          => true,
            (Command::AppendEntry { entry },                       Command::AppendEntry { entry: b }, )                                          => entry == b,
            (Command::AppendInputEntries { entries },              Command::AppendInputEntries { entries: b }, )                                 => entries == b,
            (Command::AppendBlankLog { log_id },                   Command::AppendBlankLog { log_id: b }, )                                      => log_id == b,
            (Command::ReplicateCommitted { committed },            Command::ReplicateCommitted { committed: b }, )                               => committed == b,
            (Command::LeaderCommit { already_committed, upto, },   Command::LeaderCommit { already_committed: b_committed, upto: b_upto, }, )    => already_committed == b_committed && upto == b_upto,
            (Command::FollowerCommit { already_committed, upto, }, Command::FollowerCommit { already_committed: b_committed, upto: b_upto, }, )  => already_committed == b_committed && upto == b_upto,
            (Command::Replicate { target, req },                   Command::Replicate { target: b_target, req: other_req, }, )                   => target == b_target && req == other_req,
            (Command::RebuildReplicationStreams { targets },       Command::RebuildReplicationStreams { targets: b }, )                          => targets == b,
            (Command::UpdateProgressMetrics { target, matching },  Command::UpdateProgressMetrics { target: b_target, matching: b_matching, }, ) => target == b_target && matching == b_matching,
            (Command::SaveVote { vote },                           Command::SaveVote { vote: b })                                                => vote == b,
            (Command::SendVote { vote_req },                       Command::SendVote { vote_req: b }, )                                          => vote_req == b,
            (Command::PurgeLog { upto },                           Command::PurgeLog { upto: b })                                                => upto == b,
            (Command::DeleteConflictLog { since },                 Command::DeleteConflictLog { since: b }, )                                    => since == b,
            (Command::InstallSnapshot { snapshot_meta },           Command::InstallSnapshot { snapshot_meta: b }, )                              => snapshot_meta == b,
            (Command::CancelSnapshot { snapshot_meta },            Command::CancelSnapshot { snapshot_meta: b }, )                               => snapshot_meta == b,
            (Command::Respond { resp: send },                         Command::Respond { resp: b })                                              => send == b,
            (Command::BuildSnapshot {},                            Command::BuildSnapshot {})                                                    => true,
            _ => false,
        }
    }
}

impl<NID, N, Ent> Command<NID, N, Ent>
where
    NID: NodeId,
    N: Node,
    Ent: RaftEntry<NID, N>,
{
    /// Update the flag of the metrics that needs to be updated when this command is executed.
    pub(crate) fn update_metrics_flags(&self, flags: &mut MetricsChangeFlags) {
        match &self {
            Command::BecomeLeader { .. } => flags.set_cluster_changed(),
            Command::QuitLeader => flags.set_cluster_changed(),
            Command::AppendEntry { .. } => flags.set_data_changed(),
            Command::AppendInputEntries { entries } => {
                debug_assert!(!entries.is_empty());
                flags.set_data_changed()
            }
            Command::AppendBlankLog { .. } => flags.set_data_changed(),
            Command::ReplicateCommitted { .. } => {}
            Command::LeaderCommit { .. } => flags.set_data_changed(),
            Command::FollowerCommit { .. } => flags.set_data_changed(),
            Command::Replicate { .. } => {}
            Command::RebuildReplicationStreams { .. } => flags.set_replication_changed(),
            Command::UpdateProgressMetrics { .. } => flags.set_replication_changed(),
            Command::SaveVote { .. } => flags.set_data_changed(),
            Command::SendVote { .. } => {}
            Command::PurgeLog { .. } => flags.set_data_changed(),
            Command::DeleteConflictLog { .. } => flags.set_data_changed(),
            Command::InstallSnapshot { .. } => flags.set_data_changed(),
            Command::CancelSnapshot { .. } => {}
            Command::BuildSnapshot { .. } => flags.set_data_changed(),
            Command::Respond { .. } => {}
        }
    }
}

/// A command to send return value to the caller via a `oneshot::Sender`.
#[derive(Debug)]
#[derive(PartialEq, Eq)]
#[derive(derive_more::From)]
pub(crate) enum Respond<NID, N>
where
    NID: NodeId,
    N: Node,
{
    Vote(ValueSender<Result<VoteResponse<NID>, Infallible>>),
    AppendEntries(ValueSender<Result<AppendEntriesResponse<NID>, Infallible>>),
    InstallSnapshot(ValueSender<Result<InstallSnapshotResponse<NID>, InstallSnapshotError>>),
    Initialize(ValueSender<Result<(), InitializeError<NID, N>>>),
}

impl<NID, N> Respond<NID, N>
where
    NID: NodeId,
    N: Node,
{
    pub(crate) fn new<T>(res: T, tx: oneshot::Sender<T>) -> Self
    where
        T: Debug + PartialEq + Eq,
        Self: From<ValueSender<T>>,
    {
        Respond::from(ValueSender::new(res, tx))
    }

    pub(crate) fn send(self) {
        match self {
            Respond::Vote(x) => x.send(),
            Respond::AppendEntries(x) => x.send(),
            Respond::InstallSnapshot(x) => x.send(),
            Respond::Initialize(x) => x.send(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct ValueSender<T>
where T: Debug + PartialEq + Eq
{
    value: T,
    tx: oneshot::Sender<T>,
}

impl<T> PartialEq for ValueSender<T>
where T: Debug + PartialEq + Eq
{
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<T> Eq for ValueSender<T> where T: Debug + PartialEq + Eq {}

impl<T> ValueSender<T>
where T: Debug + PartialEq + Eq
{
    pub(crate) fn new(res: T, tx: oneshot::Sender<T>) -> Self {
        Self { value: res, tx }
    }

    pub(crate) fn send(self) {
        let _ = self.tx.send(self.value);
    }
}
