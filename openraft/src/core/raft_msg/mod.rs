use std::collections::BTreeMap;

use tokio::sync::oneshot;

use crate::core::raft_msg::external_command::ExternalCommand;
use crate::error::CheckIsLeaderError;
use crate::error::ClientWriteError;
use crate::error::Infallible;
use crate::error::InitializeError;
use crate::error::InstallSnapshotError;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponse;
use crate::raft::ClientWriteResponse;
use crate::raft::InstallSnapshotRequest;
use crate::raft::InstallSnapshotResponse;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::storage::RaftLogStorage;
use crate::AsyncRuntime;
use crate::ChangeMembers;
use crate::MessageSummary;
use crate::RaftNetworkFactory;
use crate::RaftState;
use crate::RaftTypeConfig;

pub(crate) mod external_command;

/// A oneshot TX to send result from `RaftCore` to external caller, e.g. `Raft::append_entries`.
pub(crate) type ResultSender<T, E> = oneshot::Sender<Result<T, E>>;

/// TX for Install Snapshot Response
pub(crate) type InstallSnapshotTx<NID> = ResultSender<InstallSnapshotResponse<NID>, InstallSnapshotError>;

/// TX for Vote Response
pub(crate) type VoteTx<NID> = ResultSender<VoteResponse<NID>, Infallible>;

/// TX for Append Entries Response
pub(crate) type AppendEntriesTx<NID> = ResultSender<AppendEntriesResponse<NID>, Infallible>;

/// TX for Client Write Response
pub(crate) type ClientWriteTx<C> =
    ResultSender<ClientWriteResponse<C>, ClientWriteError<<C as RaftTypeConfig>::NodeId, <C as RaftTypeConfig>::Node>>;

/// A message sent by application to the [`RaftCore`].
///
/// [`RaftCore`]: crate::core::RaftCore
pub(crate) enum RaftMsg<C, N, LS>
where
    C: RaftTypeConfig,
    N: RaftNetworkFactory<C>,
    LS: RaftLogStorage<C>,
{
    AppendEntries {
        rpc: AppendEntriesRequest<C>,
        tx: AppendEntriesTx<C::NodeId>,
    },

    RequestVote {
        rpc: VoteRequest<C::NodeId>,
        tx: VoteTx<C::NodeId>,
    },

    InstallSnapshot {
        rpc: InstallSnapshotRequest<C>,
        tx: InstallSnapshotTx<C::NodeId>,
    },

    ClientWriteRequest {
        app_data: C::D,
        tx: ClientWriteTx<C>,
    },

    CheckIsLeaderRequest {
        tx: ResultSender<(), CheckIsLeaderError<C::NodeId, C::Node>>,
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

    ExternalRequest {
        #[allow(clippy::type_complexity)]
        req: Box<
            dyn FnOnce(&RaftState<C::NodeId, C::Node, <C::AsyncRuntime as AsyncRuntime>::Instant>, &mut LS, &mut N)
                + Send
                + 'static,
        >,
    },

    ExternalCommand {
        cmd: ExternalCommand,
    },
}

impl<C, N, LS> MessageSummary<RaftMsg<C, N, LS>> for RaftMsg<C, N, LS>
where
    C: RaftTypeConfig,
    N: RaftNetworkFactory<C>,
    LS: RaftLogStorage<C>,
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
            RaftMsg::ExternalRequest { .. } => "External Request".to_string(),
            RaftMsg::ExternalCommand { cmd } => {
                format!("ExternalCommand: {:?}", cmd)
            }
        }
    }
}
