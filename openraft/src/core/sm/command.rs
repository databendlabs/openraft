use std::fmt;
use std::fmt::Debug;
use std::fmt::Formatter;

use crate::base::BoxAny;
use crate::core::raft_msg::ResultSender;
use crate::error::Infallible;
use crate::raft_state::IOId;
use crate::storage::Snapshot;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::SnapshotDataOf;
use crate::RaftTypeConfig;

/// The payload of a state machine command.
pub(crate) enum Command<C>
where C: RaftTypeConfig
{
    /// Instruct the state machine to create a snapshot based on its most recent view.
    BuildSnapshot,

    /// Get the latest built snapshot.
    GetSnapshot { tx: ResultSender<C, Option<Snapshot<C>>> },

    BeginReceivingSnapshot {
        tx: ResultSender<C, SnapshotDataOf<C>, Infallible>,
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

    /// Apply a custom function to the state machine.
    ///
    /// To erase the type parameter `SM`, it is a
    /// `Box<dyn FnOnce(&mut SM) -> Box<dyn Future<Output = ()>> + Send + 'static>`
    /// wrapped in a `Box<dyn Any>`
    Func {
        func: BoxAny,
        /// The SM type user specified, for debug purpose.
        input_sm_type: &'static str,
    },
}

impl<C> Command<C>
where C: RaftTypeConfig
{
    pub(crate) fn build_snapshot() -> Self {
        Command::BuildSnapshot
    }

    pub(crate) fn get_snapshot(tx: ResultSender<C, Option<Snapshot<C>>>) -> Self {
        Command::GetSnapshot { tx }
    }

    pub(crate) fn begin_receiving_snapshot(tx: ResultSender<C, SnapshotDataOf<C>, Infallible>) -> Self {
        Command::BeginReceivingSnapshot { tx }
    }

    pub(crate) fn install_full_snapshot(snapshot: Snapshot<C>, io_id: IOId<C>) -> Self {
        Command::InstallFullSnapshot { io_id, snapshot }
    }

    /// Applies log ids within the inclusive range `[first, last]`.
    pub(crate) fn apply(first: LogIdOf<C>, last: LogIdOf<C>) -> Self {
        Command::Apply { first, last }
    }

    /// Return the IOId if this command submit any IO.
    pub(crate) fn get_submit_io(&self) -> Option<IOId<C>> {
        match self {
            Command::BuildSnapshot => None,
            Command::GetSnapshot { .. } => None,
            Command::BeginReceivingSnapshot { .. } => None,
            Command::InstallFullSnapshot { io_id, .. } => Some(io_id.clone()),
            Command::Apply { .. } => None,
            Command::Func { .. } => None,
        }
    }

    /// Return the last applied log id if this command updates the `last_applied` of the state
    /// machine.
    ///
    /// The caller can use this information to update the `apply_progress.submitted()` in `IOState`,
    /// which tracks the highest log id that has been submitted to be applied to the state machine.
    pub(crate) fn get_apply_progress(&self) -> Option<LogIdOf<C>> {
        match self {
            Command::BuildSnapshot => None,
            Command::GetSnapshot { .. } => None,
            Command::BeginReceivingSnapshot { .. } => None,
            Command::InstallFullSnapshot { io_id, .. } => io_id.last_log_id().cloned(),
            Command::Apply { last, .. } => Some(last.clone()),
            Command::Func { .. } => None,
        }
    }
}

impl<C> Debug for Command<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Command::BuildSnapshot => write!(f, "BuildSnapshot"),
            Command::GetSnapshot { .. } => write!(f, "GetSnapshot"),
            Command::InstallFullSnapshot { io_id, snapshot } => {
                write!(f, "InstallFullSnapshot: meta: {:?}, io_id: {:?}", snapshot.meta, io_id)
            }
            Command::BeginReceivingSnapshot { .. } => {
                write!(f, "BeginReceivingSnapshot")
            }
            Command::Apply { first, last } => write!(f, "Apply: [{},{}]", first, last),
            Command::Func { .. } => write!(f, "Func"),
        }
    }
}

impl<C> fmt::Display for Command<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Command::BuildSnapshot => write!(f, "BuildSnapshot"),
            Command::GetSnapshot { .. } => write!(f, "GetSnapshot"),
            Command::InstallFullSnapshot { io_id, snapshot } => {
                write!(f, "InstallFullSnapshot: meta: {}, io_id: {}", snapshot.meta, io_id)
            }
            Command::BeginReceivingSnapshot { .. } => {
                write!(f, "BeginReceivingSnapshot")
            }
            Command::Apply { first, last } => write!(f, "Apply: [{},{}]", first, last),
            Command::Func { .. } => write!(f, "Func"),
        }
    }
}

// `PartialEq` is only used for testing
impl<C> PartialEq for Command<C>
where C: RaftTypeConfig
{
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Command::BuildSnapshot, Command::BuildSnapshot) => true,
            (Command::GetSnapshot { .. }, Command::GetSnapshot { .. }) => true,
            (Command::BeginReceivingSnapshot { .. }, Command::BeginReceivingSnapshot { .. }) => true,
            (
                Command::InstallFullSnapshot {
                    io_id: io1,
                    snapshot: s1,
                },
                Command::InstallFullSnapshot {
                    io_id: io2,
                    snapshot: s2,
                },
            ) => s1.meta == s2.meta && io1 == io2,
            (
                Command::Apply { first, last },
                Command::Apply {
                    first: first2,
                    last: last2,
                },
            ) => first == first2 && last == last2,
            (Command::Func { .. }, Command::Func { .. }) => false,
            _ => false,
        }
    }
}
