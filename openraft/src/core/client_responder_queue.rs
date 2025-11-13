//! Queue for managing client responders indexed by log index.
//!
//! This module provides an optimized data structure for storing and retrieving
//! client responders associated with log entries. It's designed for the common
//! case where responders are inserted in monotonically increasing order and
//! extracted in sequential ranges from the front.

use std::collections::VecDeque;

use crate::RaftTypeConfig;
use crate::raft::responder::core_responder::CoreResponder;

/// A queue of client responders indexed by log index.
///
/// This structure is optimized for the Raft log application pattern:
/// - Responders are inserted in monotonically increasing log index order
/// - Ranges of responders are extracted sequentially (typically from the front)
/// - Extraction happens when applying committed log entries to the state machine
///
/// # Performance
///
/// - `insert()`: O(1) amortized
/// - `extract_range()`: O(log n + k) where n = total responders, k = extracted count
/// - `first_index()`, `len()`, `is_empty()`: O(1)
///
/// # Invariants
///
/// - Log indices are stored in strictly increasing order
/// - In debug builds, non-monotonic insertions will panic
pub(crate) struct ClientResponderQueue<C: RaftTypeConfig> {
    /// Responders stored as (log_index, responder) pairs in sorted order
    responders: VecDeque<(u64, CoreResponder<C>)>,
}

impl<C: RaftTypeConfig> ClientResponderQueue<C> {
    /// Creates a new empty responder queue.
    pub(crate) fn new() -> Self {
        Self {
            responders: VecDeque::new(),
        }
    }

    /// Inserts a responder for the given log index.
    ///
    /// # Requirements
    ///
    /// The log index must be greater than any previously inserted index.
    /// In debug builds, this is enforced with an assertion.
    ///
    /// # Panics
    ///
    /// In debug builds, panics if `index <= last_inserted_index`.
    pub(crate) fn insert(&mut self, index: u64, responder: CoreResponder<C>) {
        #[cfg(debug_assertions)]
        if let Some((last_index, _)) = self.responders.back() {
            debug_assert!(
                index > *last_index,
                "Responder indices must be monotonically increasing: tried to insert {} after {}",
                index,
                last_index
            );
        }

        self.responders.push_back((index, responder));
    }

    /// Extracts responders in the range [first_index, last_index] (inclusive).
    ///
    /// Returns a vector of (log_index, responder) pairs for all responders
    /// whose log index falls within the specified range. The responders are
    /// removed from the queue.
    ///
    /// # Note
    ///
    /// Not all indices in the range may have responders (e.g., membership changes,
    /// blank entries, or follower log entries don't have client responders). This
    /// is expected and handled correctly.
    ///
    /// # Performance
    ///
    /// O(log n + k) where n = total responders, k = number of responders in range.
    /// Binary search is used to find the start and end positions.
    pub(crate) fn extract_range(&mut self, first_index: u64, last_index: u64) -> Vec<(u64, CoreResponder<C>)> {
        if self.responders.is_empty() {
            return Vec::new();
        }

        // Find the start position: first responder with index >= first_index
        let start_pos = self.binary_search_start(first_index);

        // If no responders >= first_index, return empty
        if start_pos >= self.responders.len() {
            return Vec::new();
        }

        // Check if the first responder in range is already beyond last_index
        if self.responders[start_pos].0 > last_index {
            return Vec::new();
        }

        // Find the end position: last responder with index <= last_index
        let end_pos = self.find_end_position(last_index, start_pos);

        // Extract the range [start_pos, end_pos]
        self.responders.drain(start_pos..=end_pos).collect()
    }

    /// Extracts all responders from the specified index onwards.
    ///
    /// Returns a vector of all responders whose log index is >= `from_index`.
    /// The responders are removed from the queue.
    ///
    /// This is used when truncating logs - all pending responders for
    /// truncated entries need to be notified of the error.
    ///
    /// # Performance
    ///
    /// O(log n + k) where n = total responders, k = number of responders extracted.
    pub(crate) fn extract_from(&mut self, from_index: u64) -> Vec<(u64, CoreResponder<C>)> {
        if self.responders.is_empty() {
            return Vec::new();
        }

        // Find the start position: first responder with index >= from_index
        let start_pos = self.binary_search_start(from_index);

        // If no responders >= from_index, return empty
        if start_pos >= self.responders.len() {
            return Vec::new();
        }

        // Extract all responders from start_pos to end
        self.responders.drain(start_pos..).collect()
    }

    /// Returns the first (smallest) log index in the queue, if any.
    #[allow(dead_code)]
    pub(crate) fn first_index(&self) -> Option<u64> {
        self.responders.front().map(|(index, _)| *index)
    }

    /// Returns the number of responders in the queue.
    #[allow(dead_code)]
    pub(crate) fn len(&self) -> usize {
        self.responders.len()
    }

    /// Returns true if the queue is empty.
    #[allow(dead_code)]
    pub(crate) fn is_empty(&self) -> bool {
        self.responders.is_empty()
    }

    /// Binary search to find the first position with index >= target.
    ///
    /// Returns the position where the first matching element is, or
    /// responders.len() if no such element exists.
    fn binary_search_start(&self, target: u64) -> usize {
        let mut left = 0;
        let mut right = self.responders.len();

        while left < right {
            let mid = left + (right - left) / 2;
            if self.responders[mid].0 < target {
                left = mid + 1;
            } else {
                right = mid;
            }
        }

        left
    }

    /// Find the last position with index <= target, starting from start_pos.
    ///
    /// Returns the position of the last matching element.
    /// Assumes that the element at start_pos has index <= target (caller must verify).
    fn find_end_position(&self, target: u64, start_pos: usize) -> usize {
        let mut left = start_pos;
        let mut right = self.responders.len();

        while left < right {
            let mid = left + (right - left) / 2;
            if self.responders[mid].0 <= target {
                left = mid + 1;
            } else {
                right = mid;
            }
        }

        // left is now the first position with index > target
        // So the last position with index <= target is left - 1
        left - 1
    }
}

impl<C: RaftTypeConfig> Default for ClientResponderQueue<C> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::testing::UTConfig;
    use crate::raft::ClientWriteResult;
    use crate::raft::responder::ProgressResponder;
    use crate::raft::responder::core_responder::CoreResponder;

    type TestQueue = ClientResponderQueue<UTConfig>;
    type TestResponder = CoreResponder<UTConfig>;

    fn make_responder() -> TestResponder {
        let (responder, _, _): (ProgressResponder<UTConfig, ClientWriteResult<UTConfig>>, _, _) =
            ProgressResponder::new();
        TestResponder::Progress(responder)
    }

    #[test]
    fn test_new_queue_is_empty() {
        let queue = TestQueue::new();
        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);
        assert_eq!(queue.first_index(), None);
    }

    #[test]
    fn test_insert_and_basic_accessors() {
        let mut queue = TestQueue::new();

        queue.insert(10, make_responder());
        assert!(!queue.is_empty());
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.first_index(), Some(10));

        queue.insert(20, make_responder());
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.first_index(), Some(10));

        queue.insert(30, make_responder());
        assert_eq!(queue.len(), 3);
        assert_eq!(queue.first_index(), Some(10));
    }

    #[test]
    fn test_extract_simple_range() {
        let mut queue = TestQueue::new();

        queue.insert(10, make_responder());
        queue.insert(20, make_responder());
        queue.insert(30, make_responder());

        let extracted = queue.extract_range(10, 30);
        assert_eq!(extracted.len(), 3);
        assert_eq!(extracted[0].0, 10);
        assert_eq!(extracted[1].0, 20);
        assert_eq!(extracted[2].0, 30);
        assert!(queue.is_empty());
    }

    #[test]
    fn test_extract_partial_range() {
        let mut queue = TestQueue::new();

        queue.insert(10, make_responder());
        queue.insert(20, make_responder());
        queue.insert(30, make_responder());
        queue.insert(40, make_responder());

        let extracted = queue.extract_range(15, 35);
        assert_eq!(extracted.len(), 2);
        assert_eq!(extracted[0].0, 20);
        assert_eq!(extracted[1].0, 30);

        // Queue should still have 10 and 40
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.first_index(), Some(10));
    }

    #[test]
    fn test_extract_front_range() {
        let mut queue = TestQueue::new();

        queue.insert(10, make_responder());
        queue.insert(20, make_responder());
        queue.insert(30, make_responder());

        let extracted = queue.extract_range(10, 20);
        assert_eq!(extracted.len(), 2);
        assert_eq!(extracted[0].0, 10);
        assert_eq!(extracted[1].0, 20);

        assert_eq!(queue.len(), 1);
        assert_eq!(queue.first_index(), Some(30));
    }

    #[test]
    fn test_extract_middle_range() {
        let mut queue = TestQueue::new();

        queue.insert(10, make_responder());
        queue.insert(20, make_responder());
        queue.insert(30, make_responder());
        queue.insert(40, make_responder());
        queue.insert(50, make_responder());

        let extracted = queue.extract_range(20, 40);
        assert_eq!(extracted.len(), 3);
        assert_eq!(extracted[0].0, 20);
        assert_eq!(extracted[1].0, 30);
        assert_eq!(extracted[2].0, 40);

        assert_eq!(queue.len(), 2);
        assert_eq!(queue.first_index(), Some(10));
    }

    #[test]
    fn test_extract_with_gaps() {
        let mut queue = TestQueue::new();

        // Insert with gaps: 10, 20, 40, 50
        queue.insert(10, make_responder());
        queue.insert(20, make_responder());
        queue.insert(40, make_responder());
        queue.insert(50, make_responder());

        // Extract range 15-45 should get 20 and 40 (skip gaps at 30, 35)
        let extracted = queue.extract_range(15, 45);
        assert_eq!(extracted.len(), 2);
        assert_eq!(extracted[0].0, 20);
        assert_eq!(extracted[1].0, 40);

        assert_eq!(queue.len(), 2);
        assert_eq!(queue.first_index(), Some(10));
    }

    #[test]
    fn test_extract_empty_range() {
        let mut queue = TestQueue::new();

        queue.insert(10, make_responder());
        queue.insert(30, make_responder());

        // Extract range with no responders
        let extracted = queue.extract_range(15, 25);
        assert_eq!(extracted.len(), 0);

        // Queue unchanged
        assert_eq!(queue.len(), 2);
    }

    #[test]
    fn test_extract_range_before_all() {
        let mut queue = TestQueue::new();

        queue.insert(20, make_responder());
        queue.insert(30, make_responder());

        // Extract range before any responders
        let extracted = queue.extract_range(5, 15);
        assert_eq!(extracted.len(), 0);
        assert_eq!(queue.len(), 2);
    }

    #[test]
    fn test_extract_range_after_all() {
        let mut queue = TestQueue::new();

        queue.insert(10, make_responder());
        queue.insert(20, make_responder());

        // Extract range after all responders
        let extracted = queue.extract_range(30, 40);
        assert_eq!(extracted.len(), 0);
        assert_eq!(queue.len(), 2);
    }

    #[test]
    fn test_sequential_extractions() {
        let mut queue = TestQueue::new();

        queue.insert(10, make_responder());
        queue.insert(20, make_responder());
        queue.insert(30, make_responder());
        queue.insert(40, make_responder());
        queue.insert(50, make_responder());

        // First extraction
        let extracted = queue.extract_range(10, 20);
        assert_eq!(extracted.len(), 2);
        assert_eq!(queue.len(), 3);

        // Second extraction
        let extracted = queue.extract_range(30, 40);
        assert_eq!(extracted.len(), 2);
        assert_eq!(queue.len(), 1);

        // Third extraction
        let extracted = queue.extract_range(50, 50);
        assert_eq!(extracted.len(), 1);
        assert!(queue.is_empty());
    }

    #[test]
    fn test_extract_single_element() {
        let mut queue = TestQueue::new();

        queue.insert(10, make_responder());
        queue.insert(20, make_responder());
        queue.insert(30, make_responder());

        let extracted = queue.extract_range(20, 20);
        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].0, 20);
        assert_eq!(queue.len(), 2);
    }

    #[test]
    fn test_extract_exact_boundaries() {
        let mut queue = TestQueue::new();

        queue.insert(10, make_responder());
        queue.insert(20, make_responder());
        queue.insert(30, make_responder());

        // Extract with exact boundaries
        let extracted = queue.extract_range(10, 30);
        assert_eq!(extracted.len(), 3);
        assert!(queue.is_empty());
    }

    #[test]
    fn test_extract_oversized_range() {
        let mut queue = TestQueue::new();

        queue.insert(20, make_responder());
        queue.insert(30, make_responder());

        // Extract range larger than actual data
        let extracted = queue.extract_range(0, 100);
        assert_eq!(extracted.len(), 2);
        assert!(queue.is_empty());
    }

    #[test]
    fn test_multiple_inserts_and_extracts() {
        let mut queue = TestQueue::new();

        // Batch 1
        queue.insert(10, make_responder());
        queue.insert(20, make_responder());
        let extracted = queue.extract_range(10, 20);
        assert_eq!(extracted.len(), 2);
        assert!(queue.is_empty());

        // Batch 2
        queue.insert(30, make_responder());
        queue.insert(40, make_responder());
        let extracted = queue.extract_range(30, 40);
        assert_eq!(extracted.len(), 2);
        assert!(queue.is_empty());
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "Responder indices must be monotonically increasing")]
    fn test_non_monotonic_insert_panics() {
        let mut queue = TestQueue::new();

        queue.insert(20, make_responder());
        queue.insert(10, make_responder()); // Should panic
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "Responder indices must be monotonically increasing")]
    fn test_duplicate_index_panics() {
        let mut queue = TestQueue::new();

        queue.insert(20, make_responder());
        queue.insert(20, make_responder()); // Should panic
    }

    #[test]
    fn test_large_scale_operations() {
        let mut queue = TestQueue::new();

        // Insert 1000 responders
        for i in 0..1000 {
            queue.insert(i * 10, make_responder());
        }
        assert_eq!(queue.len(), 1000);

        // Extract first 500
        let extracted = queue.extract_range(0, 4990);
        assert_eq!(extracted.len(), 500);
        assert_eq!(queue.len(), 500);

        // Extract remaining
        let extracted = queue.extract_range(5000, 9990);
        assert_eq!(extracted.len(), 500);
        assert!(queue.is_empty());
    }
}
