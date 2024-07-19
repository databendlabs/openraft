use std::fmt;
use std::fmt::Debug;

use crate::async_runtime::OneshotSender;
use crate::core::sm;
use crate::display_ext::DisplayOptionExt;
use crate::display_ext::DisplayResultExt;
use crate::display_ext::DisplaySliceExt;
use crate::engine::replication_progress::ReplicationProgress;
use crate::engine::CommandKind;
use crate::error::Infallible;
use crate::error::InitializeError;
use crate::error::InstallSnapshotError;
use crate::progress::Inflight;
use crate::raft::AppendEntriesResponse;
use crate::raft::InstallSnapshotResponse;
use crate::raft::SnapshotResponse;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::raft_state::IOId;
use crate::type_config::alias::OneshotSenderOf;
use crate::vote::CommittedVote;
use crate::LogId;
use crate::OptionalSend;
use crate::RaftTypeConfig;
use crate::Vote;

/// Commands to send to `RaftRuntime` to execute, to update the application state.
#[derive(Debug)]
pub(crate) enum Command<C>
where C: RaftTypeConfig
{
    /// No actual IO is submitted but need to update io progress.
    ///
    /// For example, when Leader-1 appends log Log-2, then Leader-2 appends log Log-2 again,
    /// the second append does not need to submit to the log store,
    /// but it needs to update the IO progress from `Leader-1,Log-2` to `Leader-2,Log-2`.
    UpdateIOProgress {
        /// This command is not submit to [`RaftLogStorage`]
        /// thus it can not be queued in the log store.
        /// Therefore we need to specify the condition to wait for to run it.
        /// Usually the condition is when the previous log is flushed to disk.
        ///
        /// [`RaftLogStorage`]: crate::storage::RaftLogStorage
        when: Option<Condition<C>>,
        io_id: IOId<C>,
    },

    /// Append a `range` of entries.
    AppendInputEntries {
        /// The vote of the leader that submits the entries to write.
        ///
        /// The leader could be a local leader that appends entries to the local log store,
        /// or a remote leader that replicates entries to this follower.
        ///
        /// The leader id is used to generate a monotonic increasing IO id, such as: [`LogIOId`].
        /// Where [`LogIOId`] is `(leader_id, log_id)`.
        ///
        /// [`LogIOId`]: crate::raft_state::io_state::io_id::IOId
        committed_vote: CommittedVote<C>,

        entries: Vec<C::Entry>,
    },

    /// Replicate the committed log id to other nodes
    ReplicateCommitted { committed: Option<LogId<C::NodeId>> },

    /// Commit log entries that are already persisted in the store, upto `upto`, inclusive.
    ///
    /// To `commit` logs, [`RaftLogStorage::save_committed()`] is called. And then committed logs
    /// will be applied to the state machine by calling [`RaftStateMachine::apply()`].
    ///
    /// And if it is leader, send applied result to the client that proposed the entry.
    ///
    /// [`RaftLogStorage::save_committed()`]: crate::storage::RaftLogStorage::save_committed
    /// [`RaftStateMachine::apply()`]: crate::storage::RaftStateMachine::apply
    Commit {
        // TODO: pass the log id list or entries?
        already_committed: Option<LogId<C::NodeId>>,
        upto: LogId<C::NodeId>,
    },

    /// Replicate log entries or snapshot to a target.
    Replicate { target: C::NodeId, req: Inflight<C> },

    /// Membership config changed, need to update replication streams.
    /// The Runtime has to close all old replications and start new ones.
    /// Because a replication stream should only report state for one membership config.
    /// When membership config changes, the membership log id stored in ReplicationCore has to be
    /// updated.
    RebuildReplicationStreams {
        /// Targets to replicate to.
        targets: Vec<ReplicationProgress<C>>,
    },

    /// Save vote to storage
    SaveVote { vote: Vote<C::NodeId> },

    /// Send vote to all other members
    SendVote { vote_req: VoteRequest<C> },

    /// Purge log from the beginning to `upto`, inclusive.
    PurgeLog { upto: LogId<C::NodeId> },

    /// Delete logs that conflict with the leader from a follower/learner since log id `since`,
    /// inclusive.
    TruncateLog { since: LogId<C::NodeId> },

    // TODO(1): current it is only used to replace BuildSnapshot, InstallSnapshot, CancelSnapshot.
    /// A command send to state machine worker [`sm::worker::Worker`].
    ///
    /// The runtime(`RaftCore`) will just forward this command to [`sm::worker::Worker`].
    /// The response will be sent back in a `RaftMsg::StateMachine` message to `RaftCore`.
    StateMachine { command: sm::Command<C> },

    /// Send result to caller
    Respond {
        when: Option<Condition<C>>,
        resp: Respond<C>,
    },
}

impl<C> fmt::Display for Command<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Command::UpdateIOProgress { when, io_id } => {
                write!(f, "UpdateIOProgress: when: {}, io_id: {}", when.display(), io_id)
            }
            Command::AppendInputEntries {
                committed_vote: vote,
                entries,
            } => {
                write!(f, "AppendInputEntries: vote: {}, entries: {}", vote, entries.display())
            }
            Command::ReplicateCommitted { committed } => {
                write!(f, "ReplicateCommitted: {}", committed.display())
            }
            Command::Commit {
                already_committed,
                upto,
            } => write!(f, "Commit: ({}, {}]", already_committed.display(), upto),
            Command::Replicate { target, req } => {
                write!(f, "Replicate: target={}, req: {}", target, req)
            }
            Command::RebuildReplicationStreams { targets } => {
                write!(f, "RebuildReplicationStreams: {}", targets.display_n::<10>())
            }
            Command::SaveVote { vote } => write!(f, "SaveVote: {}", vote),
            Command::SendVote { vote_req } => write!(f, "SendVote: {}", vote_req),
            Command::PurgeLog { upto } => write!(f, "PurgeLog: upto: {}", upto),
            Command::TruncateLog { since } => write!(f, "TruncateLog: since: {}", since),
            Command::StateMachine { command } => write!(f, "StateMachine: command: {}", command),
            Command::Respond { when, resp } => write!(f, "Respond: when: {}, resp: {}", when.display(), resp),
        }
    }
}

impl<C> From<sm::Command<C>> for Command<C>
where C: RaftTypeConfig
{
    fn from(cmd: sm::Command<C>) -> Self {
        Self::StateMachine { command: cmd }
    }
}

/// For unit testing
impl<C> PartialEq for Command<C>
where
    C: RaftTypeConfig,
    C::Entry: PartialEq,
{
    #[rustfmt::skip]
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Command::UpdateIOProgress { when, io_id },        Command::UpdateIOProgress { when: wb, io_id: ab }, )                  => when == wb && io_id == ab,
            (Command::AppendInputEntries { committed_vote: vote, entries },    Command::AppendInputEntries { committed_vote: vb, entries: b }, )               => vote == vb && entries == b,
            (Command::ReplicateCommitted { committed },        Command::ReplicateCommitted { committed: b }, )                       => committed == b,
            (Command::Commit { already_committed, upto, },     Command::Commit { already_committed: b_committed, upto: b_upto, }, )  => already_committed == b_committed && upto == b_upto,
            (Command::Replicate { target, req },               Command::Replicate { target: b_target, req: other_req, }, )           => target == b_target && req == other_req,
            (Command::RebuildReplicationStreams { targets },   Command::RebuildReplicationStreams { targets: b }, )                  => targets == b,
            (Command::SaveVote { vote },                       Command::SaveVote { vote: b })                                        => vote == b,
            (Command::SendVote { vote_req },                   Command::SendVote { vote_req: b }, )                                  => vote_req == b,
            (Command::PurgeLog { upto },                       Command::PurgeLog { upto: b })                                        => upto == b,
            (Command::TruncateLog { since },                   Command::TruncateLog { since: b }, )                                        => since == b,
            (Command::Respond { when, resp: send },            Command::Respond { when: b_when, resp: b })                           => send == b && when == b_when,
            (Command::StateMachine { command },                Command::StateMachine { command: b })                                 => command == b,
            _ => false,
        }
    }
}

impl<C> Command<C>
where C: RaftTypeConfig
{
    #[allow(dead_code)]
    #[rustfmt::skip]
    pub(crate) fn kind(&self) -> CommandKind {
        match self {
            Command::RebuildReplicationStreams { .. } => CommandKind::Main,
            Command::Respond { .. }                   => CommandKind::Main,

            Command::UpdateIOProgress { .. }            => CommandKind::Log,
            Command::AppendInputEntries { .. }        => CommandKind::Log,
            Command::SaveVote { .. }                  => CommandKind::Log,
            Command::PurgeLog { .. }                  => CommandKind::Log,
            Command::TruncateLog { .. }               => CommandKind::Log,

            Command::ReplicateCommitted { .. }        => CommandKind::Network,
            Command::Replicate { .. }                 => CommandKind::Network,
            Command::SendVote { .. }                  => CommandKind::Network,

            Command::StateMachine { .. }              => CommandKind::StateMachine,
            // Apply is firstly handled by RaftCore, then forwarded to state machine worker.
            // TODO: Apply also write `committed` to log-store, which should be run in CommandKind::Log
            Command::Commit { .. }                    => CommandKind::Main,
        }
    }

    /// Return the condition the command waits for if any.
    #[rustfmt::skip]
    pub(crate) fn condition(&self) -> Option<Condition<C>> {
        match self {
            Command::UpdateIOProgress { when, .. }    => *when,
            Command::AppendInputEntries { .. }        => None,
            Command::ReplicateCommitted { .. }        => None,
            // TODO: Apply also write `committed` to log-store, which should be run in CommandKind::Log
            Command::Commit { .. }                    => None,
            Command::Replicate { .. }                 => None,
            Command::RebuildReplicationStreams { .. } => None,
            Command::SaveVote { .. }                  => None,
            Command::SendVote { .. }                  => None,
            Command::PurgeLog { upto }                => Some(Condition::Snapshot { log_id: Some(*upto) }),
            Command::TruncateLog { .. }               => None,
            Command::Respond { when, .. }             => *when,
            Command::StateMachine { .. }              => None,
        }
    }
}

/// A condition to wait for before running a command.
#[derive(Debug, Clone, Copy)]
#[derive(PartialEq, Eq)]
pub(crate) enum Condition<C>
where C: RaftTypeConfig
{
    /// Wait until the log is flushed to the disk.
    ///
    /// In raft, a log io can be uniquely identified by `(leader_id, log_id)`, not `log_id`.
    /// A same log id can be written multiple times by different leaders.
    IOFlushed { io_id: IOId<C> },

    /// A log without specific leader is flushed to disk.
    ///
    /// This is only used by [`Raft::initialize()`], because when initializing there is no leader.
    ///
    /// [`Raft::initialize()`]: `crate::Raft::initialize()`
    LogFlushed { log_id: Option<LogId<C::NodeId>> },

    /// Wait until the log is applied to the state machine.
    #[allow(dead_code)]
    Applied { log_id: Option<LogId<C::NodeId>> },

    /// Wait until snapshot is built and includes the log id.
    Snapshot { log_id: Option<LogId<C::NodeId>> },
}

impl<C> fmt::Display for Condition<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Condition::IOFlushed { io_id } => {
                write!(f, "IOFlushed: {}", io_id)
            }
            Condition::LogFlushed { log_id } => {
                write!(f, "LogFlushed: {}", log_id.display())
            }
            Condition::Applied { log_id } => write!(f, "Applied: log_id: {}", log_id.display()),
            Condition::Snapshot { log_id } => write!(f, "Snapshot: log_id: {}", log_id.display()),
        }
    }
}

/// A command to send return value to the caller via a `oneshot::Sender`.
#[derive(Debug, PartialEq, Eq)]
#[derive(derive_more::From)]
pub(crate) enum Respond<C>
where C: RaftTypeConfig
{
    Vote(ValueSender<C, Result<VoteResponse<C>, Infallible>>),
    AppendEntries(ValueSender<C, Result<AppendEntriesResponse<C>, Infallible>>),
    ReceiveSnapshotChunk(ValueSender<C, Result<(), InstallSnapshotError>>),
    InstallSnapshot(ValueSender<C, Result<InstallSnapshotResponse<C>, InstallSnapshotError>>),
    InstallFullSnapshot(ValueSender<C, Result<SnapshotResponse<C>, Infallible>>),
    Initialize(ValueSender<C, Result<(), InitializeError<C>>>),
}

impl<C> fmt::Display for Respond<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Respond::Vote(vs) => write!(f, "Vote {}", vs.value().display()),
            Respond::AppendEntries(vs) => write!(f, "AppendEntries {}", vs.value().display()),
            Respond::ReceiveSnapshotChunk(vs) => {
                write!(
                    f,
                    "ReceiveSnapshotChunk {}",
                    vs.value().as_ref().map(|_x| "()").display()
                )
            }
            Respond::InstallSnapshot(vs) => write!(f, "InstallSnapshot {}", vs.value().display()),
            Respond::InstallFullSnapshot(vs) => write!(f, "InstallFullSnapshot {}", vs.value().display()),
            Respond::Initialize(vs) => write!(f, "Initialize {}", vs.value().as_ref().map(|_x| "()").display()),
        }
    }
}

impl<C> Respond<C>
where C: RaftTypeConfig
{
    pub(crate) fn new<T>(res: T, tx: OneshotSenderOf<C, T>) -> Self
    where
        T: Debug + PartialEq + Eq + OptionalSend,
        Self: From<ValueSender<C, T>>,
    {
        Respond::from(ValueSender::new(res, tx))
    }

    pub(crate) fn send(self) {
        match self {
            Respond::Vote(x) => x.send(),
            Respond::AppendEntries(x) => x.send(),
            Respond::ReceiveSnapshotChunk(x) => x.send(),
            Respond::InstallSnapshot(x) => x.send(),
            Respond::InstallFullSnapshot(x) => x.send(),
            Respond::Initialize(x) => x.send(),
        }
    }
}

pub(crate) struct ValueSender<C, T>
where
    T: Debug + PartialEq + Eq + OptionalSend,
    C: RaftTypeConfig,
{
    value: T,
    tx: OneshotSenderOf<C, T>,
}

impl<C, T> Debug for ValueSender<C, T>
where
    T: Debug + PartialEq + Eq + OptionalSend,
    C: RaftTypeConfig,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ValueSender").field("value", &self.value).finish()
    }
}

impl<C, T> PartialEq for ValueSender<C, T>
where
    T: Debug + PartialEq + Eq + OptionalSend,
    C: RaftTypeConfig,
{
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<C, T> Eq for ValueSender<C, T>
where
    T: Debug + PartialEq + Eq + OptionalSend,
    C: RaftTypeConfig,
{
}

impl<C, T> ValueSender<C, T>
where
    T: Debug + PartialEq + Eq + OptionalSend,
    C: RaftTypeConfig,
{
    pub(crate) fn new(res: T, tx: OneshotSenderOf<C, T>) -> Self {
        Self { value: res, tx }
    }

    pub(crate) fn value(&self) -> &T {
        &self.value
    }

    pub(crate) fn send(self) {
        let _ = self.tx.send(self.value);
    }
}
