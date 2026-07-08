//! RPC request type enum for all Raft RPC calls.

use std::fmt;

use openraft::OptionalSend;
use openraft::RPCTypes;
use openraft::RaftTypeConfig;
use openraft::alias::SnapshotOf;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::InstallSnapshotRequest;
use openraft::raft::TransferLeaderRequest;
use openraft::raft::VoteRequest;

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
    TransferLeader(TransferLeaderRequest<C>),
}

impl<C, SD> RpcRequest<C, SD>
where
    C: RaftTypeConfig,
    SD: fmt::Debug + OptionalSend + 'static,
{
    pub fn get_type(&self) -> RPCTypes {
        match self {
            RpcRequest::AppendEntries(_) => RPCTypes::AppendEntries,
            RpcRequest::InstallSnapshot(_) => RPCTypes::InstallSnapshot,
            RpcRequest::InstallFullSnapshot(_) => RPCTypes::InstallSnapshot,
            RpcRequest::Vote(_) => RPCTypes::Vote,
            RpcRequest::TransferLeader(_) => RPCTypes::TransferLeader,
        }
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
            RpcRequest::TransferLeader(req) => write!(f, "TransferLeader({})", req),
        }
    }
}
