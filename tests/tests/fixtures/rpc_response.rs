//! RPC response type enum for all Raft RPC calls.

use std::fmt;

use openraft::RPCTypes;
use openraft::RaftTypeConfig;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::InstallSnapshotResponse;
use openraft::raft::SnapshotResponse;
use openraft::raft::VoteResponse;

/// Unified enum for all RPC response types in the test framework.
#[derive(Debug)]
#[derive(derive_more::From, derive_more::TryInto)]
pub enum RpcResponse<C>
where
    C: RaftTypeConfig,
    C::SnapshotData: fmt::Debug,
{
    AppendEntries(AppendEntriesResponse<C>),
    InstallSnapshot(InstallSnapshotResponse<C>),
    InstallFullSnapshot(SnapshotResponse<C>),
    Vote(VoteResponse<C>),
    TransferLeader(()),
}

impl<C> RpcResponse<C>
where
    C: RaftTypeConfig,
    C::SnapshotData: fmt::Debug,
{
    pub fn get_type(&self) -> RPCTypes {
        match self {
            RpcResponse::AppendEntries(_) => RPCTypes::AppendEntries,
            RpcResponse::InstallSnapshot(_) => RPCTypes::InstallSnapshot,
            RpcResponse::InstallFullSnapshot(_) => RPCTypes::InstallSnapshot,
            RpcResponse::Vote(_) => RPCTypes::Vote,
            RpcResponse::TransferLeader(_) => RPCTypes::TransferLeader,
        }
    }
}

impl<C> fmt::Display for RpcResponse<C>
where
    C: RaftTypeConfig,
    C::SnapshotData: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RpcResponse::AppendEntries(resp) => write!(f, "AppendEntries({})", resp),
            RpcResponse::InstallSnapshot(req) => write!(f, "InstallSnapshot({})", req),
            RpcResponse::InstallFullSnapshot(resp) => write!(f, "InstallFullSnapshot({})", resp),
            RpcResponse::Vote(resp) => write!(f, "Vote({})", resp),
            RpcResponse::TransferLeader(_resp) => write!(f, "TransferLeader(())"),
        }
    }
}
