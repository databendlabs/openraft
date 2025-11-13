//! Queue of client responders indexed by log index, optimized for sequential
//! insertion and range extraction.

use std::collections::VecDeque;

/// Queue of client responders indexed by log index.
///
/// Responders must be pushed in monotonically increasing order.
/// In debug builds, non-monotonic pushes panic.
///
/// # Performance
///
/// - `push()`: O(1) amortized
/// - `extend()`: O(k) where k = items added
/// - `drain_upto()`: O(k) where k = drained count
/// - `drain_from()`: O(log n + k) where n = total responders, k = drained count
/// - `first_index()`, `len()`, `is_empty()`: O(1)
pub(crate) struct ClientResponderQueue<T> {
    /// Responders stored as (log_index, responder) pairs in sorted order
    responders: VecDeque<(u64, T)>,
}

impl<T> ClientResponderQueue<T> {
    /// Creates a new empty responder queue.
    pub(crate) fn new() -> Self {
        Self {
            responders: VecDeque::new(),
        }
    }

    /// Creates a new empty responder queue with the specified capacity.
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            responders: VecDeque::with_capacity(capacity),
        }
    }

    /// Pushes a responder to the end of the queue.
    ///
    /// # Panics
    ///
    /// In debug builds, panics if `index <= last_index`.
    pub(crate) fn push(&mut self, index: u64, responder: T) {
        #[cfg(debug_assertions)]
        if let Some((last_index, _)) = self.responders.back() {
            debug_assert!(
                index > *last_index,
                "Responder indices must be monotonically increasing: tried to push {} after {}",
                index,
                last_index
            );
        }

        self.responders.push_back((index, responder));
    }

    /// Extends the queue with multiple responders.
    ///
    /// # Panics
    ///
    /// In debug builds, panics if indices are not monotonically increasing.
    #[allow(dead_code)]
    pub(crate) fn extend(&mut self, iter: impl IntoIterator<Item = (u64, T)>) {
        for (index, responder) in iter {
            self.push(index, responder);
        }
    }

    /// Drains responders from the beginning up to and including `last_index`.
    ///
    /// Returns matching responders and removes them from the queue.
    /// Not all indices in the range may have responders.
    pub(crate) fn drain_upto(&mut self, last_index: u64) -> Vec<(u64, T)> {
        let end_pos = self.responders.partition_point(|(index, _)| *index <= last_index);
        self.responders.drain(0..end_pos).collect()
    }

    /// Drains all responders from the specified index onwards.
    ///
    /// Returns an iterator over responders with `log_index >= from_index` and removes them.
    /// Used when truncating logs to notify affected responders.
    pub(crate) fn drain_from(&mut self, from_index: u64) -> impl Iterator<Item = (u64, T)> + '_ {
        let start_pos =
            self.responders.binary_search_by_key(&from_index, |(index, _)| *index).unwrap_or_else(|pos| pos);

        self.responders.drain(start_pos..)
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
}

impl<T> Default for ClientResponderQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestQueue = ClientResponderQueue<u64>;

    #[test]
    fn test_new() {
        let queue = TestQueue::new();
        assert!(queue.is_empty());
        assert_eq!(queue.first_index(), None);
    }

    #[test]
    fn test_push() {
        let mut queue = TestQueue::new();
        queue.push(10, 100);
        queue.push(20, 200);
        queue.push(30, 300);

        assert_eq!(queue.len(), 3);
        assert_eq!(queue.first_index(), Some(10));
        assert_eq!(queue.drain_upto(30), vec![(10, 100), (20, 200), (30, 300)]);
    }

    #[test]
    fn test_extend() {
        let mut queue = TestQueue::new();
        queue.extend(vec![(10, 100), (20, 200), (30, 300)]);
        assert_eq!(queue.drain_upto(30), vec![(10, 100), (20, 200), (30, 300)]);

        queue.push(40, 400);
        queue.extend(vec![(50, 500), (60, 600)]);
        assert_eq!(queue.drain_upto(60), vec![(40, 400), (50, 500), (60, 600)]);
    }

    #[test]
    fn test_drain_upto() {
        let mut queue = TestQueue::new();
        queue.push(10, 100);
        queue.push(20, 200);
        queue.push(30, 300);
        queue.push(40, 400);

        // Partial drain
        assert_eq!(queue.drain_upto(25), vec![(10, 100), (20, 200)]);
        assert_eq!(queue.first_index(), Some(30));

        // Drain with gaps
        queue.push(60, 600);
        assert_eq!(queue.drain_upto(50), vec![(30, 300), (40, 400)]);
        assert_eq!(queue.first_index(), Some(60));

        // Drain all
        assert_eq!(queue.drain_upto(100), vec![(60, 600)]);
        assert!(queue.is_empty());

        // Drain empty
        assert_eq!(queue.drain_upto(100), vec![]);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "Responder indices must be monotonically increasing")]
    fn test_push_panics_non_monotonic() {
        let mut queue = TestQueue::new();
        queue.push(20, 200);
        queue.push(10, 100);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "Responder indices must be monotonically increasing")]
    fn test_push_panics_duplicate() {
        let mut queue = TestQueue::new();
        queue.push(20, 200);
        queue.push(20, 200);
    }

    #[test]
    fn test_large_scale() {
        let mut queue = TestQueue::new();
        for i in 0..1000 {
            queue.push(i * 10, i * 10);
        }

        assert_eq!(queue.drain_upto(4990).len(), 500);
        assert_eq!(queue.drain_upto(9990).len(), 500);
        assert!(queue.is_empty());
    }

    #[test]
    fn test_drain_from() {
        let mut queue = TestQueue::new();
        queue.push(10, 100);
        queue.push(20, 200);
        queue.push(30, 300);
        queue.push(40, 400);

        // Drain from middle
        let drained: Vec<_> = queue.drain_from(25).collect();
        assert_eq!(drained, vec![(30, 300), (40, 400)]);
        assert_eq!(queue.first_index(), Some(10));

        // Drain from exact match
        let drained: Vec<_> = queue.drain_from(10).collect();
        assert_eq!(drained, vec![(10, 100), (20, 200)]);
        assert!(queue.is_empty());

        // Drain from empty
        let drained: Vec<_> = queue.drain_from(50).collect();
        assert_eq!(drained, vec![]);

        // Early break
        queue.extend(vec![(10, 100), (20, 200), (30, 300)]);
        let mut result = Vec::new();
        for (log_index, value) in queue.drain_from(20) {
            result.push((log_index, value));
            if log_index == 20 {
                break;
            }
        }
        assert_eq!(result, vec![(20, 200)]);
        assert_eq!(queue.first_index(), Some(10));
    }
}
