use std::fmt;
use std::fmt::Formatter;

use crate::core::ApplyResult;
use crate::display_ext::display_result::DisplayResultExt;
use crate::display_ext::DisplayOptionExt;
use crate::raft_state::IOId;
use crate::storage::SnapshotMeta;
use crate::RaftTypeConfig;
use crate::StorageError;

/// The Ok part of a state machine command result.
#[derive(Debug)]
pub(crate) enum Response<C>
where C: RaftTypeConfig
{
    /// Build a snapshot, it returns result via the universal RaftCore response channel.
    BuildSnapshot(SnapshotMeta<C>),

    /// When finishing installing a snapshot.
    ///
    /// It does not return any value to RaftCore.
    InstallSnapshot((IOId<C>, Option<SnapshotMeta<C>>)),

    /// Send back applied result to RaftCore.
    Apply(ApplyResult<C>),
}

impl<C> fmt::Display for Response<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::BuildSnapshot(meta) => {
                write!(f, "BuildSnapshot({})", meta)
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
