//! Module defining Raft node ID as a GUID.
//!
//! This depends on the configuration of the feature `guid_nodeid`. If set, then a larger, 16-byte
//! NodeId storing a GUID will be used (defined here).

use std::fmt::Debug;
use std::fmt::Display;

use serde::Deserialize;
use serde::Serialize;

/// A Raft node's ID.
///
/// The node ID is encoded as a GUID (more concretely, UUID variant 1, big-endian).
#[derive(Default, Copy, Clone, Eq, PartialEq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NodeId {
    first: u64,
    second: u64,
}

impl NodeId {
    /// Create a new, invalid `NodeId`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a `NodeId` from raw values.
    #[must_use]
    pub const fn new_from_parts(first: u64, second: u64) -> Self {
        Self {
            first: first.to_be(),
            second: second.to_be(),
        }
    }

    /// Check if the `NodeId` is valid.
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        self.first != 0 || self.second != 0
    }

    /// Get the first 64 bits.
    #[must_use]
    pub const fn first(&self) -> u64 {
        u64::from_be(self.first)
    }

    /// Get the second 64 bits.
    #[must_use]
    pub const fn second(&self) -> u64 {
        u64::from_be(self.second)
    }

    /// Determine first and second parts of this `NodeId`.
    #[must_use]
    pub const fn into_parts(&self) -> (u64, u64) {
        (self.first(), self.second())
    }
}

impl Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (first, second) = self.into_parts();
        f.write_fmt(format_args!(
            "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
            first >> 32,
            (first >> 16) & 0xffff,
            first & 0xffff,
            second >> 48,
            second & 0xffff_ffff_ffff
        ))
    }
}

impl Debug for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty() {
        let empty = NodeId::new();
        assert_eq!(empty, NodeId::default());
        assert!(!empty.is_valid());
    }

    #[test]
    fn nonempty() {
        let node_id = NodeId::new_from_parts(1, 2);
        assert_ne!(node_id, NodeId::default());
        assert_eq!(node_id, node_id);
    }

    #[test]
    fn format() {
        let node_id = NodeId::new_from_parts(0x0123_4567_89ab_8cde, 0xf123_def1_cccc_0ddd);
        let str = format!("{}", node_id);
        assert_eq!(str, "01234567-89ab-8cde-f123-def1cccc0ddd");
    }
}
