use std::fmt;
use std::fmt::Debug;
use std::fmt::Formatter;

use crate::core::raft_msg::ResultSender;
use crate::error::Infallible;
use crate::raft_state::IOId;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::SnapshotDataOf;
use crate::RaftTypeConfig;
use crate::Snapshot;

#[derive(PartialEq)]
pub(crate) struct Command<C>
where C: RaftTypeConfig
{
    pub(crate) payload: CommandPayload<C>,
}

impl<C> Debug for Command<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("StateMachineCommand").field("payload", &self.payload).finish()
    }
}

impl<C> fmt::Display for Command<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "sm::Command: payload: {}", self.payload)
    }
}

impl<C> Command<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(payload: CommandPayload<C>) -> Self {
        Self { payload }
    }

    pub(crate) fn build_snapshot() -> Self {
        let payload = CommandPayload::BuildSnapshot;
        Command::new(payload)
    }

    pub(crate) fn get_snapshot(tx: ResultSender<C, Option<Snapshot<C>>>) -> Self {
        let payload = CommandPayload::GetSnapshot { tx };
        Command::new(payload)
    }

    pub(crate) fn begin_receiving_snapshot(tx: ResultSender<C, Box<SnapshotDataOf<C>>, Infallible>) -> Self {
        let payload = CommandPayload::BeginReceivingSnapshot { tx };
        Command::new(payload)
    }

    pub(crate) fn install_full_snapshot(snapshot: Snapshot<C>, io_id: IOId<C>) -> Self {
        let payload = CommandPayload::InstallFullSnapshot { io_id, snapshot };
        Command::new(payload)
    }

    /// Applies log ids within the inclusive range `[first, last]`.
    pub(crate) fn apply(first: LogIdOf<C>, last: LogIdOf<C>) -> Self {
        let payload = CommandPayload::Apply { first, last };
        Command::new(payload)
    }

    /// Return the IOId if this command submit any IO.
    pub(crate) fn get_submit_io(&self) -> Option<IOId<C>> {
        match self.payload {
            CommandPayload::BuildSnapshot => None,
            CommandPayload::GetSnapshot { .. } => None,
            CommandPayload::BeginReceivingSnapshot { .. } => None,
            CommandPayload::InstallFullSnapshot { io_id, .. } => Some(io_id),
            CommandPayload::Apply { .. } => None,
        }
    }
}

/// The payload of a state machine command.
pub(crate) enum CommandPayload<C>
where C: RaftTypeConfig
{
    /// Instruct the state machine to create a snapshot based on its most recent view.
    BuildSnapshot,

    /// Get the latest built snapshot.
    GetSnapshot { tx: ResultSender<C, Option<Snapshot<C>>> },

    BeginReceivingSnapshot {
        tx: ResultSender<C, Box<SnapshotDataOf<C>>, Infallible>,
    },

    InstallFullSnapshot {
        /// The IO id used to update IO progress.
        ///
        /// Installing a snapshot is considered as an IO of AppendEntries `[0,
        /// snapshot.last_log_id]`
        io_id: IOId<C>,
        snapshot: Snapshot<C>,
    },

    /// Apply the log entries to the state machine.
    Apply {
        /// The first log id to apply, inclusive.
        first: LogIdOf<C>,

        /// The last log id to apply, inclusive.
        last: LogIdOf<C>,
    },
}

impl<C> Debug for CommandPayload<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            CommandPayload::BuildSnapshot => write!(f, "BuildSnapshot"),
            CommandPayload::GetSnapshot { .. } => write!(f, "GetSnapshot"),
            CommandPayload::InstallFullSnapshot { io_id, snapshot } => {
                write!(f, "InstallFullSnapshot: meta: {:?}, io_id: {:?}", snapshot.meta, io_id)
            }
            CommandPayload::BeginReceivingSnapshot { .. } => {
                write!(f, "BeginReceivingSnapshot")
            }
            CommandPayload::Apply { first, last } => write!(f, "Apply: [{},{}]", first, last),
        }
    }
}

impl<C> fmt::Display for CommandPayload<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            CommandPayload::BuildSnapshot => write!(f, "BuildSnapshot"),
            CommandPayload::GetSnapshot { .. } => write!(f, "GetSnapshot"),
            CommandPayload::InstallFullSnapshot { io_id, snapshot } => {
                write!(f, "InstallFullSnapshot: meta: {}, io_id: {}", snapshot.meta, io_id)
            }
            CommandPayload::BeginReceivingSnapshot { .. } => {
                write!(f, "BeginReceivingSnapshot")
            }
            CommandPayload::Apply { first, last } => write!(f, "Apply: [{},{}]", first, last),
        }
    }
}

// `PartialEq` is only used for testing
impl<C> PartialEq for CommandPayload<C>
where C: RaftTypeConfig
{
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (CommandPayload::BuildSnapshot, CommandPayload::BuildSnapshot) => true,
            (CommandPayload::GetSnapshot { .. }, CommandPayload::GetSnapshot { .. }) => true,
            (CommandPayload::BeginReceivingSnapshot { .. }, CommandPayload::BeginReceivingSnapshot { .. }) => true,
            (
                CommandPayload::InstallFullSnapshot {
                    io_id: io1,
                    snapshot: s1,
                },
                CommandPayload::InstallFullSnapshot {
                    io_id: io2,
                    snapshot: s2,
                },
            ) => s1.meta == s2.meta && io1 == io2,
            (
                CommandPayload::Apply { first, last },
                CommandPayload::Apply {
                    first: first2,
                    last: last2,
                },
            ) => first == first2 && last == last2,
            _ => false,
        }
    }
}
