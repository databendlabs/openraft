//! RPC request type enum for all Raft RPC calls.

use std::fmt;

use openraft::OptionalSend;
use openraft::RPCTypes;
use openraft::RaftTypeConfig;
use openraft::alias::SnapshotOf;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::TransferLeaderRequest;
use openraft::raft::VoteRequest;
use openraft_legacy::network_v1::InstallSnapshotRequest;

use crate::fixtures::rpc_kind::RpcKind;

/// Unified enum for all RPC request types in the test framework.
#[derive(Debug)]
#[derive(derive_more::From, derive_more::TryInto)]
pub enum RpcRequest<C, SD = ()>
where
    C: RaftTypeConfig,
    SD: fmt::Debug + OptionalSend + 'static,
{
    AppendEntries(AppendEntriesRequest<C>),
    InstallSnapshot(InstallSnapshotRequest<C>),
    InstallFullSnapshot(SnapshotOf<C, SD>),
    Vote(VoteRequest<C>),
    #[from(skip)]
    #[try_into(ignore)]
    PreVote(VoteRequest<C>),
    TransferLeader(TransferLeaderRequest<C>),
}

impl<C, SD> RpcRequest<C, SD>
where
    C: RaftTypeConfig,
    SD: fmt::Debug + OptionalSend + 'static,
{
    /// Return the exact RPC kind, preserving the election phase.
    pub fn kind(&self) -> RpcKind {
        match self {
            Self::AppendEntries(_) => RpcKind::AppendEntries,
            Self::InstallSnapshot(_) => RpcKind::InstallSnapshot,
            Self::InstallFullSnapshot(_) => RpcKind::InstallFullSnapshot,
            Self::Vote(_) => RpcKind::Vote,
            Self::PreVote(_) => RpcKind::PreVote,
            Self::TransferLeader(_) => RpcKind::TransferLeader,
        }
    }

    pub fn get_type(&self) -> RPCTypes {
        let kind = self.kind();
        kind.rpc_type()
    }
}

impl<C, SD> fmt::Display for RpcRequest<C, SD>
where
    C: RaftTypeConfig,
    SD: fmt::Debug + OptionalSend + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RpcRequest::AppendEntries(req) => write!(f, "AppendEntries({})", req),
            RpcRequest::InstallSnapshot(req) => write!(f, "InstallSnapshot({})", req),
            RpcRequest::InstallFullSnapshot(req) => write!(f, "InstallFullSnapshot({})", req.meta),
            RpcRequest::Vote(req) => write!(f, "Vote({})", req),
            RpcRequest::PreVote(req) => write!(f, "PreVote({})", req),
            RpcRequest::TransferLeader(req) => write!(f, "TransferLeader({})", req),
        }
    }
}
