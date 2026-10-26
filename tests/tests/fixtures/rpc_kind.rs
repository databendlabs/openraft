use openraft::RPCTypes;

/// Identifies the exact RPC, including the two election phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RpcKind {
    AppendEntries,
    InstallSnapshot,
    InstallFullSnapshot,
    Vote,
    PreVote,
    TransferLeader,
}

impl RpcKind {
    /// Return the production API's grouped RPC type.
    pub fn rpc_type(self) -> RPCTypes {
        match self {
            Self::AppendEntries => RPCTypes::AppendEntries,
            Self::InstallSnapshot | Self::InstallFullSnapshot => RPCTypes::InstallSnapshot,
            Self::Vote | Self::PreVote => RPCTypes::Vote,
            Self::TransferLeader => RPCTypes::TransferLeader,
        }
    }
}
