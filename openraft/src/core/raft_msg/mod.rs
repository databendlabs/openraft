use std::collections::BTreeMap;
use std::fmt;

use crate::ChangeMembers;
use crate::RaftState;
use crate::RaftTypeConfig;
use crate::base::BoxOnce;
use crate::core::raft_msg::external_command::ExternalCommand;
use crate::display_ext::DisplayBTreeMapDebugValueExt;
use crate::error::CheckIsLeaderError;
use crate::error::Infallible;
use crate::error::InitializeError;
use crate::impls::ProgressResponder;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponse;
use crate::raft::ClientWriteResult;
use crate::raft::ReadPolicy;
use crate::raft::SnapshotResponse;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::raft::linearizable_read::Linearizer;
use crate::raft::responder::core_responder::CoreResponder;
use crate::storage::Snapshot;
use crate::type_config::alias::OneshotSenderOf;
use crate::type_config::alias::SnapshotDataOf;
use crate::type_config::alias::VoteOf;

pub(crate) mod external_command;

/// A oneshot TX to send result from `RaftCore` to external caller, e.g. `Raft::append_entries`.
pub(crate) type ResultSender<C, T, E = Infallible> = OneshotSenderOf<C, Result<T, E>>;

/// TX for Vote Response
pub(crate) type VoteTx<C> = OneshotSenderOf<C, VoteResponse<C>>;

/// TX for Append Entries Response
pub(crate) type AppendEntriesTx<C> = OneshotSenderOf<C, AppendEntriesResponse<C>>;

/// TX for Linearizable Read Response
pub(crate) type ClientReadTx<C> = ResultSender<C, Linearizer<C>, CheckIsLeaderError<C>>;

/// A message sent by application to the [`RaftCore`].
///
/// [`RaftCore`]: crate::core::RaftCore
pub(crate) enum RaftMsg<C>
where C: RaftTypeConfig
{
    AppendEntries {
        rpc: AppendEntriesRequest<C>,
        tx: AppendEntriesTx<C>,
    },

    RequestVote {
        rpc: VoteRequest<C>,
        tx: VoteTx<C>,
    },

    InstallFullSnapshot {
        vote: VoteOf<C>,
        snapshot: Snapshot<C>,
        tx: OneshotSenderOf<C, SnapshotResponse<C>>,
    },

    /// Begin receiving a snapshot from the leader.
    ///
    /// Returns a snapshot data handle for receiving data.
    ///
    /// It does not check `Vote` because it is a read operation
    /// and does not break raft protocol.
    BeginReceivingSnapshot {
        tx: OneshotSenderOf<C, SnapshotDataOf<C>>,
    },

    ClientWriteRequest {
        app_data: C::D,
        responder: Option<CoreResponder<C>>,
    },

    CheckIsLeaderRequest {
        read_policy: ReadPolicy,
        tx: ClientReadTx<C>,
    },

    Initialize {
        members: BTreeMap<C::NodeId, C::Node>,
        tx: ResultSender<C, (), InitializeError<C>>,
    },

    ChangeMembership {
        changes: ChangeMembers<C>,

        /// If `retain` is `true`, then the voters that are not in the new
        /// config will be converted into learners, otherwise they will be removed.
        retain: bool,

        tx: ProgressResponder<C, ClientWriteResult<C>>,
    },

    ExternalCoreRequest {
        req: BoxOnce<'static, RaftState<C>>,
    },

    /// Transfer Leader to another node.
    ///
    /// If this node is `to`, reset Leader lease and start election.
    /// Otherwise, just reset Leader lease so that the node `to` can become Leader.
    HandleTransferLeader {
        /// The vote of the Leader that is transferring the leadership.
        from: VoteOf<C>,
        /// The assigned node to be the next Leader.
        to: C::NodeId,
    },

    ExternalCommand {
        cmd: ExternalCommand<C>,
    },
}

impl<C> fmt::Display for RaftMsg<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RaftMsg::AppendEntries { rpc, .. } => {
                write!(f, "AppendEntries: {}", rpc)
            }
            RaftMsg::RequestVote { rpc, .. } => {
                write!(f, "RequestVote: {}", rpc)
            }
            RaftMsg::BeginReceivingSnapshot { .. } => {
                write!(f, "BeginReceivingSnapshot")
            }
            RaftMsg::InstallFullSnapshot { vote, snapshot, .. } => {
                write!(f, "InstallFullSnapshot: vote: {}, snapshot: {}", vote, snapshot)
            }
            RaftMsg::ClientWriteRequest { .. } => write!(f, "ClientWriteRequest"),
            RaftMsg::CheckIsLeaderRequest { read_policy, .. } => {
                write!(f, "CheckIsLeaderRequest with read policy: {}", read_policy)
            }
            RaftMsg::Initialize { members, .. } => {
                write!(f, "Initialize: {}", members.display())
            }
            RaftMsg::ChangeMembership { changes, retain, .. } => {
                write!(f, "ChangeMembership: {}, retain: {}", changes, retain)
            }
            RaftMsg::ExternalCoreRequest { .. } => write!(f, "External Request"),
            RaftMsg::HandleTransferLeader { from, to } => {
                write!(f, "TransferLeader: from_leader: vote={}, to: {}", from, to)
            }
            RaftMsg::ExternalCommand { cmd } => {
                write!(f, "ExternalCommand: {}", cmd)
            }
        }
    }
}
