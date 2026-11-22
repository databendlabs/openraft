use std::fmt;
use std::fmt::Formatter;

use display_more::DisplayOptionExt;
use display_more::DisplayResultExt;

use crate::RaftTypeConfig;
use crate::StorageError;
use crate::core::ApplyResult;
use crate::raft_state::io_state::log_io_id::LogIOId;
use crate::type_config::alias::SnapshotMetaOf;

/// The Ok part of a state machine command result.
#[derive(Debug)]
pub(crate) enum Response<C>
where C: RaftTypeConfig
{
    /// Snapshot building completed or was deferred.
    ///
    /// - `Some(meta)`: Snapshot was successfully built with the given metadata.
    /// - `None`: State machine deferred snapshot creation via `try_create_snapshot_builder()`.
    BuildSnapshotDone(Option<SnapshotMetaOf<C>>),

    /// A full snapshot is installed.
    ///
    /// It carries the log I/O id that the installation completes, and the installed snapshot's
    /// metadata. RaftCore flushes log progress up to that id, and apply and snapshot progress up to
    /// the snapshot's last log id, which releases the reply to the installation request and lets
    /// the logs the snapshot covers be purged.
    InstallSnapshot((LogIOId<C>, Option<SnapshotMetaOf<C>>)),

    /// Send back applied result to RaftCore.
    Apply(ApplyResult<C>),
}

impl<C> fmt::Display for Response<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::BuildSnapshotDone(meta) => {
                write!(f, "BuildSnapshotDone({})", meta.display())
            }
            Self::InstallSnapshot((io_id, meta)) => {
                write!(f, "InstallSnapshot(io_id:{}, meta:{})", io_id, meta.display())
            }
            Self::Apply(result) => {
                write!(f, "{}", result)
            }
        }
    }
}

/// Container of result of a command.
#[derive(Debug)]
pub(crate) struct CommandResult<C>
where C: RaftTypeConfig
{
    pub(crate) result: Result<Response<C>, StorageError<C>>,
}

impl<C> fmt::Display for CommandResult<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "sm::Result({})", self.result.display())
    }
}

impl<C> CommandResult<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(result: Result<Response<C>, StorageError<C>>) -> Self {
        Self { result }
    }
}
