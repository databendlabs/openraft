use std::fmt;

use openraft_macros::since;

/// Types of RPC requests in the Raft protocol.
#[since(version = "0.10.2", change = "added PreVote variant")]
#[since(version = "0.10.1", change = "marked #[non_exhaustive]")]
#[derive(Debug, Clone, Copy)]
#[derive(PartialEq, Eq)]
#[derive(Hash)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[non_exhaustive]
pub enum RPCTypes {
    /// Vote request RPC.
    Vote,
    /// AppendEntries request RPC.
    AppendEntries,
    /// InstallSnapshot request RPC.
    InstallSnapshot,
    /// TransferLeader request RPC.
    #[since(version = "0.10.0")]
    TransferLeader,
    // Add new variants last: a binary format such as bincode encodes a variant by its index.
    /// PreVote request RPC.
    #[since(version = "0.10.2")]
    PreVote,
}

impl fmt::Display for RPCTypes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[cfg(test)]
mod tests {
    use super::RPCTypes;

    #[test]
    fn test_display() {
        assert_eq!(RPCTypes::Vote.to_string(), "Vote");
        assert_eq!(RPCTypes::PreVote.to_string(), "PreVote");
    }

    #[cfg(feature = "serde")]
    #[test]
    fn test_serialized_variants_stay_compatible() {
        let cases = [
            (RPCTypes::Vote, "\"Vote\"", 0u32),
            (RPCTypes::AppendEntries, "\"AppendEntries\"", 1),
            (RPCTypes::InstallSnapshot, "\"InstallSnapshot\"", 2),
            (RPCTypes::TransferLeader, "\"TransferLeader\"", 3),
            (RPCTypes::PreVote, "\"PreVote\"", 4),
        ];

        for (rpc, name, index) in cases {
            assert_eq!(serde_json::to_string(&rpc).unwrap(), name);
            assert_eq!(bincode::serialize(&rpc).unwrap(), index.to_le_bytes());
        }
    }
}
