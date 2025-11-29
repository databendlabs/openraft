//! Shard Router - Routes requests to the correct shard based on key range.
//!
//! In a sharded system, each shard is responsible for a range of keys. The shard router
//! maintains the mapping from key ranges to shard IDs and routes requests accordingly.
//!
//! ## How It Works
//!
//! The router maintains a list of shards, each with:
//! - A shard ID
//! - A key range (start, end) that this shard is responsible for
//!
//! When a request comes in, the router:
//! 1. Extracts the user_id from the key
//! 2. Finds the shard whose range contains this user_id
//! 3. Returns the shard ID for routing
//!
//! ## Split Updates
//!
//! After a split operation, the router must be updated:
//! - The original shard's range is shrunk (end = split_point)
//! - A new shard entry is added (start = split_point + 1, end = old_end)
//!
//! ```text
//! Before Split:
//! ┌─────────────────────────────────────┐
//! │ shard_a: [1, MAX]                   │
//! └─────────────────────────────────────┘
//!
//! After Split at 100:
//! ┌─────────────────────────────────────┐
//! │ shard_a: [1, 100]                   │
//! │ shard_b: [101, MAX]                 │
//! └─────────────────────────────────────┘
//! ```

use std::sync::RwLock;

use crate::ShardId;

/// A key range that a shard is responsible for.
///
/// The range is inclusive on both ends: [start, end].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRange {
    /// The start of the range (inclusive).
    pub start: u64,
    /// The end of the range (inclusive).
    pub end: u64,
}

impl KeyRange {
    /// Create a new key range.
    pub fn new(start: u64, end: u64) -> Self {
        Self { start, end }
    }

    /// Check if the range contains the given key.
    pub fn contains(&self, key: u64) -> bool {
        key >= self.start && key <= self.end
    }
}

/// A shard entry in the router.
#[derive(Debug, Clone)]
pub struct ShardEntry {
    /// The shard ID.
    pub shard_id: ShardId,
    /// The key range this shard is responsible for.
    pub range: KeyRange,
}

/// Routes requests to the correct shard based on key range.
///
/// This is the central routing component in a sharded system. It maintains
/// the mapping from key ranges to shards and must be kept in sync with
/// the actual shard topology.
///
/// ## Thread Safety
///
/// The router uses interior mutability (RwLock) to allow concurrent reads
/// and exclusive writes. This is important because:
/// - Reads (routing) are very frequent and should be fast
/// - Writes (split updates) are rare but must be atomic
#[derive(Debug, Default)]
pub struct ShardRouter {
    /// The list of shards and their key ranges.
    shards: RwLock<Vec<ShardEntry>>,
}

impl ShardRouter {
    /// Create a new empty shard router.
    pub fn new() -> Self {
        Self {
            shards: RwLock::new(Vec::new()),
        }
    }

    /// Create a shard router with an initial shard covering all keys.
    ///
    /// This is typically used when starting a new cluster with a single shard.
    pub fn with_initial_shard(shard_id: ShardId) -> Self {
        let router = Self::new();
        router.add_shard(shard_id, KeyRange::new(1, u64::MAX));
        router
    }

    /// Add a new shard with the given key range.
    pub fn add_shard(&self, shard_id: ShardId, range: KeyRange) {
        let mut shards = self.shards.write().unwrap();
        shards.push(ShardEntry { shard_id, range });
    }

    /// Route a key to the appropriate shard.
    pub fn route(&self, user_id: u64) -> Option<ShardId> {
        let shards = self.shards.read().unwrap();
        for entry in shards.iter() {
            if entry.range.contains(user_id) {
                return Some(entry.shard_id.clone());
            }
        }
        None
    }

    /// Update the router after a split operation.
    ///
    /// This method:
    /// 1. Shrinks the original shard's range to [start, split_at]
    /// 2. Adds a new shard entry for [split_at + 1, old_end]
    ///
    /// # Arguments
    /// * `original_shard` - The ID of the shard being split
    /// * `split_at` - The split point (keys > split_at go to new shard)
    /// * `new_shard_id` - The ID of the new shard
    ///
    /// # Returns
    /// `true` if the split was applied, `false` if the original shard wasn't found.
    pub fn apply_split(&self, original_shard: &str, split_at: u64, new_shard_id: ShardId) -> bool {
        let mut shards = self.shards.write().unwrap();

        // Find the original shard
        let original_idx = shards.iter().position(|e| e.shard_id == original_shard);

        if let Some(idx) = original_idx {
            let original_end = shards[idx].range.end;

            // Shrink the original shard's range
            shards[idx].range.end = split_at;

            // Add the new shard
            shards.push(ShardEntry {
                shard_id: new_shard_id,
                range: KeyRange::new(split_at + 1, original_end),
            });

            true
        } else {
            false
        }
    }

    /// Get all shard entries.
    pub fn all_shards(&self) -> Vec<ShardEntry> {
        let shards = self.shards.read().unwrap();
        shards.clone()
    }

    /// Get the key range for a specific shard.
    pub fn get_range(&self, shard_id: &str) -> Option<KeyRange> {
        let shards = self.shards.read().unwrap();
        shards.iter().find(|e| e.shard_id == shard_id).map(|e| e.range.clone())
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_range_contains() {
        let range = KeyRange::new(1, 100);
        assert!(range.contains(1));
        assert!(range.contains(50));
        assert!(range.contains(100));
        assert!(!range.contains(0));
        assert!(!range.contains(101));
    }

    #[test]
    fn test_initial_shard() {
        let router = ShardRouter::with_initial_shard("shard_a".to_string());

        assert_eq!(router.route(1), Some("shard_a".to_string()));
        assert_eq!(router.route(1000), Some("shard_a".to_string()));
        assert_eq!(router.route(u64::MAX), Some("shard_a".to_string()));
    }

    #[test]
    fn test_apply_split() {
        let router = ShardRouter::with_initial_shard("shard_a".to_string());

        // Split at 100
        assert!(router.apply_split("shard_a", 100, "shard_b".to_string()));

        // Verify routing
        assert_eq!(router.route(1), Some("shard_a".to_string()));
        assert_eq!(router.route(100), Some("shard_a".to_string()));
        assert_eq!(router.route(101), Some("shard_b".to_string()));
        assert_eq!(router.route(1000), Some("shard_b".to_string()));

        // Verify ranges
        let range_a = router.get_range("shard_a").unwrap();
        assert_eq!(range_a.start, 1);
        assert_eq!(range_a.end, 100);

        let range_b = router.get_range("shard_b").unwrap();
        assert_eq!(range_b.start, 101);
        assert_eq!(range_b.end, u64::MAX);
    }

    #[test]
    fn test_split_nonexistent_shard() {
        let router = ShardRouter::new();
        assert!(!router.apply_split("nonexistent", 100, "new_shard".to_string()));
    }
}
