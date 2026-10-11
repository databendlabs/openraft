use std::ops::BitOr;
use std::ops::BitOrAssign;

use macros::since;

/// Node capabilities that can be combined with `|`.
///
/// Read and write voter status is determined by the membership's quorum sets.
#[since(version = "0.10.0")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct NodeRole(u8);

#[allow(non_upper_case_globals)]
impl NodeRole {
    /// The node has no campaign or log replication capability.
    #[since(version = "0.10.0")]
    pub const Empty: Self = Self(0);

    /// Allows the node to start an election on its own.
    ///
    /// A leadership transfer authorized by the current leader may start an election on a node
    /// without this capability. The target must still have an up-to-date log and win the election
    /// quorum.
    #[since(version = "0.10.0")]
    pub const Campaigner: Self = Self(1 << 0);

    /// Allows the node to receive replicated logs.
    ///
    /// Voters and campaigners can also have this capability.
    #[since(version = "0.10.0")]
    pub const ReceiveLogs: Self = Self(1 << 1);

    /// Returns whether all capabilities in `role` are present.
    #[since(version = "0.10.0")]
    pub const fn contains(self, role: Self) -> bool {
        let common = self.0 & role.0;
        common == role.0
    }
}

impl BitOr for NodeRole {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        let roles = self.0 | rhs.0;
        Self(roles)
    }
}

impl BitOrAssign for NodeRole {
    fn bitor_assign(&mut self, rhs: Self) {
        let combined = *self | rhs;
        *self = combined;
    }
}

#[cfg(test)]
mod tests {
    use super::NodeRole;

    #[test]
    fn combine_roles() {
        let combined = NodeRole::Campaigner | NodeRole::ReceiveLogs;
        let campaigner = combined.contains(NodeRole::Campaigner);
        let receives_logs = combined.contains(NodeRole::ReceiveLogs);
        assert!(campaigner);
        assert!(receives_logs);

        let reversed = NodeRole::ReceiveLogs | NodeRole::Campaigner;
        assert_eq!(combined, reversed);

        let repeated = combined | NodeRole::Campaigner;
        assert_eq!(combined, repeated);

        let with_empty = combined | NodeRole::Empty;
        assert_eq!(combined, with_empty);

        let empty_first = NodeRole::Empty | combined;
        assert_eq!(combined, empty_first);
    }

    #[test]
    fn single_role_and_assignment() {
        let mut role = NodeRole::Empty;
        let campaigner = role.contains(NodeRole::Campaigner);
        let receives_logs = role.contains(NodeRole::ReceiveLogs);
        assert!(!campaigner);
        assert!(!receives_logs);

        role |= NodeRole::Campaigner;
        let receives_logs = role.contains(NodeRole::ReceiveLogs);
        assert!(!receives_logs);

        role |= NodeRole::ReceiveLogs;
        let campaigner = role.contains(NodeRole::Campaigner);
        let receives_logs = role.contains(NodeRole::ReceiveLogs);
        assert!(campaigner);
        assert!(receives_logs);
    }
}
