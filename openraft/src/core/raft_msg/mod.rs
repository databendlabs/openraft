use std::collections::BTreeMap;

use tokio::sync::oneshot;

use crate::core::raft_msg::external_command::ExternalCommand;
use crate::error::CheckIsLeaderError;
use crate::error::ClientWriteError;
use crate::error::HigherVote;
use crate::error::Infallible;
use crate::error::InitializeError;
use crate::error::InstallSnapshotError;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponse;
use crate::raft::BoxCoreFn;
use crate::raft::ClientWriteResponse;
use crate::raft::InstallSnapshotRequest;
use crate::raft::InstallSnapshotResponse;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::NodeIdOf;
use crate::type_config::alias::NodeOf;
use crate::type_config::alias::SnapshotDataOf;
use crate::ChangeMembers;
use crate::MessageSummary;
use crate::RaftTypeConfig;
use crate::Snapshot;
use crate::Vote;

pub(crate) mod external_command;

/// A oneshot TX to send result from `RaftCore` to external caller, e.g. `Raft::append_entries`.
pub(crate) type ResultSender<T, E = Infallible> = oneshot::Sender<Result<T, E>>;

pub(crate) type ResultReceiver<T, E = Infallible> = oneshot::Receiver<Result<T, E>>;

/// TX for Install Snapshot Response
pub(crate) type InstallSnapshotTx<NID> = ResultSender<InstallSnapshotResponse<NID>, InstallSnapshotError>;

/// TX for Vote Response
pub(crate) type VoteTx<NID> = ResultSender<VoteResponse<NID>>;

/// TX for Append Entries Response
pub(crate) type AppendEntriesTx<NID> = ResultSender<AppendEntriesResponse<NID>>;

/// TX for Client Write Response
pub(crate) type ClientWriteTx<C> = ResultSender<ClientWriteResponse<C>, ClientWriteError<NodeIdOf<C>, NodeOf<C>>>;

/// TX for Linearizable Read Response
pub(crate) type ClientReadTx<C> =
    ResultSender<(Option<LogIdOf<C>>, Option<LogIdOf<C>>), CheckIsLeaderError<NodeIdOf<C>, NodeOf<C>>>;

/// A message sent by application to the [`RaftCore`].
///
/// [`RaftCore`]: crate::core::RaftCore
pub(crate) enum RaftMsg<C>
where C: RaftTypeConfig
{
    AppendEntries {
        rpc: AppendEntriesRequest<C>,
        tx: AppendEntriesTx<C::NodeId>,
    },

    RequestVote {
        rpc: VoteRequest<C::NodeId>,
        tx: VoteTx<C::NodeId>,
    },

    // TODO: remove
    #[allow(dead_code)]
    InstallSnapshot {
        rpc: InstallSnapshotRequest<C>,
        tx: InstallSnapshotTx<C::NodeId>,
    },

    InstallCompleteSnapshot {
        vote: Vote<C::NodeId>,
        snapshot: Snapshot<C>,
        tx: ResultSender<InstallSnapshotResponse<C::NodeId>>,
    },

    /// Begin receiving a snapshot from the leader.
    ///
    /// Returns a handle to a snapshot data ready for receiving if successful.
    /// Otherwise, it is an error because of the `vote` is not GE the local `vote`, the local `vote`
    /// will be returned in a Err
    BeginReceiveSnapshot {
        vote: Vote<C::NodeId>,
        tx: ResultSender<Box<SnapshotDataOf<C>>, HigherVote<C::NodeId>>,
    },

    ClientWriteRequest {
        app_data: C::D,
        tx: ClientWriteTx<C>,
    },

    CheckIsLeaderRequest {
        tx: ClientReadTx<C>,
    },

    Initialize {
        members: BTreeMap<C::NodeId, C::Node>,
        tx: ResultSender<(), InitializeError<C::NodeId, C::Node>>,
    },

    ChangeMembership {
        changes: ChangeMembers<C::NodeId, C::Node>,

        /// If `retain` is `true`, then the voters that are not in the new
        /// config will be converted into learners, otherwise they will be removed.
        retain: bool,

        tx: ResultSender<ClientWriteResponse<C>, ClientWriteError<C::NodeId, C::Node>>,
    },

    ExternalCoreRequest {
        req: BoxCoreFn<C>,
    },

    ExternalCommand {
        cmd: ExternalCommand<C>,
    },
}

impl<C> MessageSummary<RaftMsg<C>> for RaftMsg<C>
where C: RaftTypeConfig
{
    fn summary(&self) -> String {
        match self {
            RaftMsg::AppendEntries { rpc, .. } => {
                format!("AppendEntries: {}", rpc.summary())
            }
            RaftMsg::RequestVote { rpc, .. } => {
                format!("RequestVote: {}", rpc.summary())
            }
            RaftMsg::InstallSnapshot { rpc, .. } => {
                format!("InstallSnapshot: {}", rpc.summary())
            }
            RaftMsg::BeginReceiveSnapshot { vote, .. } => {
                format!("BeginReceiveSnapshot: vote: {}", vote)
            }
            RaftMsg::InstallCompleteSnapshot { vote, snapshot, .. } => {
                format!("InstallCompleteSnapshot: vote: {}, snapshot: {}", vote, snapshot)
            }
            RaftMsg::ClientWriteRequest { .. } => "ClientWriteRequest".to_string(),
            RaftMsg::CheckIsLeaderRequest { .. } => "CheckIsLeaderRequest".to_string(),
            RaftMsg::Initialize { members, .. } => {
                format!("Initialize: {:?}", members)
            }
            RaftMsg::ChangeMembership {
                changes: members,
                retain,
                ..
            } => {
                format!("ChangeMembership: members: {:?}, retain: {}", members, retain,)
            }
            RaftMsg::ExternalCoreRequest { .. } => "External Request".to_string(),
            RaftMsg::ExternalCommand { cmd } => {
                format!("ExternalCommand: {:?}", cmd)
            }
        }
    }
}
