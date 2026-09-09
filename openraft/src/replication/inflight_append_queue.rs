use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;

use display_more::DisplayOptionExt;

use crate::RaftTypeConfig;
use crate::replication::inflight_append::InflightAppend;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::LogIdOf;

/// A queue tracking in-flight AppendEntries requests for measuring replication latency.
///
/// When an AppendEntries request is sent, its metadata is pushed to this queue.
/// When a response arrives with a matching log id, all requests up to and including
/// that log id are drained, and the sending time of the last drained request is returned
/// for RTT calculation.
#[derive(Clone)]
pub(crate) struct InflightAppendQueue<C>
where C: RaftTypeConfig
{
    queue: Arc<Mutex<VecDeque<InflightAppend<C>>>>,
}

impl<C> InflightAppendQueue<C>
where C: RaftTypeConfig
{
    pub(crate) fn new() -> Self {
        Self {
            queue: Arc::new(Mutex::new(VecDeque::with_capacity(32))),
        }
    }

    /// Records a new in-flight AppendEntries request.
    pub(crate) fn push(&self, prev_log_id: Option<LogIdOf<C>>, last_log_id: Option<LogIdOf<C>>) {
        let mut q = self.queue.lock().unwrap();
        let inflight = InflightAppend::new(prev_log_id, last_log_id);

        tracing::debug!("Inflight queue push: {}", inflight);

        q.push_back(inflight)
    }

    /// Removes stale requests through the first request at or containing a conflict point and
    /// returns that request's sending time.
    ///
    /// At a shared boundary, the request starting at the conflict point is selected.
    pub(crate) fn drain_conflicted(&self, conflict_log_id: &LogIdOf<C>) -> Option<InstantOf<C>> {
        let mut q = self.queue.lock().unwrap();

        while let Some(first) = q.front() {
            let starts_at_conflict = first.prev_log_id.as_ref() == Some(conflict_log_id);
            let ends_at_or_before_conflict = first.last_log_id.as_ref() <= Some(conflict_log_id);
            let is_stale = !starts_at_conflict && ends_at_or_before_conflict;
            if is_stale {
                q.pop_front();
                continue;
            }

            let contains_conflict = first.prev_log_id.as_ref() < Some(conflict_log_id);
            let matches_conflict = starts_at_conflict || contains_conflict;
            if matches_conflict {
                let sending_time = first.sending_time;
                q.pop_front();
                return Some(sending_time);
            }

            break;
        }

        None
    }

    /// Removes all requests with `last_log_id <= matching` and returns
    /// the sending time of the last removed request.
    ///
    /// Returns `None` if no requests were removed.
    pub(crate) fn drain_acked(&self, matching: &Option<LogIdOf<C>>) -> Option<InstantOf<C>> {
        let mut q = self.queue.lock().unwrap();

        tracing::debug!(
            "Inflight queue drain_acked: matching: {}; data: {:?}",
            matching.display(),
            q.as_slices()
        );

        let mut last = None;
        while let Some(first) = q.front() {
            if matching >= &first.last_log_id {
                last = Some(first.sending_time)
            } else {
                break;
            }

            q.pop_front();
        }

        last
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::testing::UTConfig;
    use crate::engine::testing::log_id;

    #[test]
    fn test_new() {
        let q = InflightAppendQueue::<UTConfig>::new();
        assert_eq!(q.drain_acked(&None), None);
    }

    #[test]
    fn test_drain_conflicted() {
        let q = InflightAppendQueue::<UTConfig>::new();
        q.push(Some(log_id(1, 1, 0)), Some(log_id(1, 1, 10)));
        q.push(Some(log_id(1, 1, 10)), Some(log_id(1, 1, 10)));
        q.push(Some(log_id(1, 1, 10)), Some(log_id(1, 1, 30)));

        let (boundary_time, interior_time) = {
            let mut queue = q.queue.lock().unwrap();
            let boundary_time = queue[0].sending_time + std::time::Duration::from_secs(1);
            queue[1].sending_time = boundary_time;
            let interior_time = boundary_time + std::time::Duration::from_secs(1);
            queue[2].sending_time = interior_time;
            (boundary_time, interior_time)
        };

        // Conflict(10) rejects B's prev, even when A's last is also 10.
        let actual = q.drain_conflicted(&log_id(1, 1, 10));
        assert_eq!(actual, Some(boundary_time));

        {
            let queue = q.queue.lock().unwrap();
            assert_eq!(queue.len(), 1);
            assert_eq!(queue[0].last_log_id, Some(log_id(1, 1, 30)));
        }

        // Conflict(25) falls inside C's range.
        let actual = q.drain_conflicted(&log_id(1, 1, 25));
        assert_eq!(actual, Some(interior_time));

        {
            let queue = q.queue.lock().unwrap();
            assert!(queue.is_empty());
        }

        let actual = q.drain_conflicted(&log_id(1, 1, 30));
        assert_eq!(actual, None);
    }

    #[test]
    fn test_push_and_drain_acked_none_matching() {
        let q = InflightAppendQueue::<UTConfig>::new();
        q.push(None, Some(log_id(1, 1, 5)));
        q.push(Some(log_id(1, 1, 5)), Some(log_id(1, 1, 10)));

        // matching=None is less than any log_id, so nothing is acked
        assert_eq!(q.drain_acked(&None), None);

        // Queue should remain unchanged
        assert_eq!(q.queue.lock().unwrap().len(), 2);
    }

    #[test]
    fn test_drain_acked_partial() {
        let q = InflightAppendQueue::<UTConfig>::new();
        q.push(None, Some(log_id(1, 1, 5)));
        q.push(Some(log_id(1, 1, 5)), Some(log_id(1, 1, 10)));
        q.push(Some(log_id(1, 1, 10)), Some(log_id(1, 1, 15)));

        // Read the expected sending_time before calling drain_acked
        let expected_time = q.queue.lock().unwrap()[1].sending_time;

        // matching=10 should ack entries with last_log_id <= 10
        let result = q.drain_acked(&Some(log_id(1, 1, 10)));

        // Should return the sending_time of entry with log_id=10
        assert_eq!(result, Some(expected_time));

        // Two entries removed (5 and 10), one remains (15)
        let deque = q.queue.lock().unwrap();
        assert_eq!(deque.len(), 1);
        assert_eq!(deque[0].last_log_id, Some(log_id(1, 1, 15)));
    }

    #[test]
    fn test_drain_acked_all() {
        let q = InflightAppendQueue::<UTConfig>::new();
        q.push(None, Some(log_id(1, 1, 5)));
        q.push(Some(log_id(1, 1, 5)), Some(log_id(1, 1, 10)));

        // Read the expected sending_time before calling drain_acked
        let expected_time = q.queue.lock().unwrap()[1].sending_time;

        // matching=20 is greater than all entries
        let result = q.drain_acked(&Some(log_id(1, 1, 20)));

        // Should return the sending_time of the last entry (log_id=10)
        assert_eq!(result, Some(expected_time));

        // All entries should be removed
        assert_eq!(q.queue.lock().unwrap().len(), 0);
    }

    #[test]
    fn test_drain_acked_empty_queue() {
        let q = InflightAppendQueue::<UTConfig>::new();
        assert_eq!(q.drain_acked(&Some(log_id(1, 1, 10))), None);
    }

    #[test]
    fn test_drain_acked_with_none_log_id() {
        let q = InflightAppendQueue::<UTConfig>::new();
        q.push(None, None);
        q.push(None, Some(log_id(1, 1, 5)));

        // Read the expected sending_time before calling drain_acked
        let expected_time = q.queue.lock().unwrap()[0].sending_time;

        // matching=None should ack entries with last_log_id <= None (i.e., only None)
        let result = q.drain_acked(&None);

        // Should return the sending_time of entry with None log_id
        assert_eq!(result, Some(expected_time));

        // Entry with None removed, entry with 5 remains
        let deque = q.queue.lock().unwrap();
        assert_eq!(deque.len(), 1);
        assert_eq!(deque[0].last_log_id, Some(log_id(1, 1, 5)));
    }
}
