use std::fmt;

use crate::RaftTypeConfig;
use crate::raft::SnapshotResponse;
use crate::storage::RaftStateMachine;
use crate::type_config::alias::OneshotSenderOf;
use crate::type_config::alias::SmSnapshotOf;
use crate::type_config::alias::VoteOf;

/// A request to install a full snapshot, sent to [`RaftCore`] via a dedicated channel.
///
/// The snapshot data type is defined by the state machine, thus this request does not go
/// through the [`RaftMsg`] channel, which is independent of the state machine type.
///
/// [`RaftCore`]: crate::core::RaftCore
/// [`RaftMsg`]: crate::core::raft_msg::RaftMsg
pub(crate) struct InstallFullSnapshotRequest<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    pub(crate) vote: VoteOf<C>,
    pub(crate) snapshot: SmSnapshotOf<C, SM>,
    pub(crate) tx: OneshotSenderOf<C, SnapshotResponse<C>>,
}

impl<C, SM> fmt::Display for InstallFullSnapshotRequest<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "InstallFullSnapshot: vote: {}, snapshot: {}",
            self.vote, self.snapshot
        )
    }
}
