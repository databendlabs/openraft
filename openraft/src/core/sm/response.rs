use std::fmt;
use std::fmt::Formatter;

use crate::core::sm::command::CommandSeq;
use crate::core::ApplyResult;
use crate::display_ext::display_result::DisplayResultExt;
use crate::display_ext::DisplayOptionExt;
use crate::RaftTypeConfig;
use crate::SnapshotMeta;
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
    InstallSnapshot(Option<SnapshotMeta<C>>),

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
            Self::InstallSnapshot(meta) => {
                write!(f, "InstallSnapshot({})", meta.display())
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
    #[allow(dead_code)]
    pub(crate) command_seq: CommandSeq,
    pub(crate) result: Result<Response<C>, StorageError<C>>,
}

impl<C> fmt::Display for CommandResult<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "sm::Result(command_seq:{}, {})",
            self.command_seq,
            self.result.display()
        )
    }
}

impl<C> CommandResult<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(command_seq: CommandSeq, result: Result<Response<C>, StorageError<C>>) -> Self {
        Self { command_seq, result }
    }
}
