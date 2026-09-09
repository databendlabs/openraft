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
/// that log id are drained. The sending time of the last fully or partially acknowledged
/// request is returned for RTT calculation.
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

    /// Returns a conservative sending time for a request rejected with `Conflict`.
    ///
    /// `Conflict` identifies the rejected request by its `prev_log_id`. Returning the earliest
    /// match is conservative when multiple requests share the same `prev_log_id`.
    /// The queue is discarded when the conflict terminates the stream.
    pub(crate) fn sending_time_for_conflict(&self, conflict_log_id: &LogIdOf<C>) -> Option<InstantOf<C>> {
        let q = self.queue.lock().unwrap();

        q.iter()
            .find(|inflight| inflight.prev_log_id.as_ref() == Some(conflict_log_id))
            .map(|inflight| inflight.sending_time)
    }

    /// Removes all requests with `last_log_id <= matching` and returns
    /// the sending time of the last fully or partially acknowledged request.
    ///
    /// A request with `prev_log_id < matching < last_log_id` contributes its sending time
    /// but remains queued until its last log id is acknowledged.
    ///
    /// Returns `None` if no request is fully or partially acknowledged.
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
                if matching > &first.prev_log_id {
                    last = Some(first.sending_time);
                }
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
    fn test_sending_time_for_conflict() {
        let q = InflightAppendQueue::<UTConfig>::new();
        q.push(Some(log_id(1, 1, 0)), Some(log_id(1, 1, 10)));
        q.push(Some(log_id(1, 1, 10)), Some(log_id(1, 1, 20)));

        let expected_time = {
            let mut queue = q.queue.lock().unwrap();
            let expected_time = queue[0].sending_time + std::time::Duration::from_secs(1);
            queue[1].sending_time = expected_time;
            expected_time
        };

        // Conflict(10) rejects B's prev, even when A's last is also 10.
        assert_eq!(q.sending_time_for_conflict(&log_id(1, 1, 10)), Some(expected_time));
        assert_eq!(q.sending_time_for_conflict(&log_id(1, 1, 30)), None);
    }

    #[test]
    fn test_push_and_drain_acked_none_matching() {
        let q = InflightAppendQueue::<UTConfig>::new();
        q.push(None, Some(log_id(1, 1, 5)));
        q.push(Some(log_id(1, 1, 5)), Some(log_id(1, 1, 10)));

        // matching=None does not fully acknowledge either request.
        assert_eq!(q.drain_acked(&None), None);
        assert_eq!(q.queue.lock().unwrap().len(), 2);
    }

    #[test]
    fn test_drain_acked_inside_request() {
        let q = InflightAppendQueue::<UTConfig>::new();
        q.push(Some(log_id(1, 1, 0)), Some(log_id(1, 1, 10)));
        q.push(Some(log_id(1, 1, 10)), Some(log_id(1, 1, 20)));

        let expected_time = q.queue.lock().unwrap()[0].sending_time;

        assert_eq!(q.drain_acked(&Some(log_id(1, 1, 5))), Some(expected_time));
        assert_eq!(q.queue.lock().unwrap().len(), 2);

        assert_eq!(q.drain_acked(&Some(log_id(1, 1, 10))), Some(expected_time));
        assert_eq!(q.queue.lock().unwrap().len(), 1);
        assert_eq!(q.queue.lock().unwrap()[0].last_log_id, Some(log_id(1, 1, 20)));
        assert_eq!(q.drain_acked(&Some(log_id(1, 1, 10))), None);
    }

    #[test]
    fn test_drain_acked_cumulative_partial() {
        let q = InflightAppendQueue::<UTConfig>::new();
        q.push(Some(log_id(1, 1, 0)), Some(log_id(1, 1, 10)));
        q.push(Some(log_id(1, 1, 10)), Some(log_id(1, 1, 20)));

        let expected_time = {
            let mut queue = q.queue.lock().unwrap();
            let expected_time = queue[0].sending_time + std::time::Duration::from_secs(1);
            queue[1].sending_time = expected_time;
            expected_time
        };

        assert_eq!(q.drain_acked(&Some(log_id(1, 1, 15))), Some(expected_time));
        let deque = q.queue.lock().unwrap();
        assert_eq!(deque.len(), 1);
        assert_eq!(deque[0].last_log_id, Some(log_id(1, 1, 20)));
    }

    #[test]
    fn test_drain_acked_same_last_log_id() {
        let q = InflightAppendQueue::<UTConfig>::new();
        q.push(None, Some(log_id(1, 1, 10)));
        q.push(Some(log_id(1, 1, 10)), Some(log_id(1, 1, 10)));
        q.push(Some(log_id(1, 1, 10)), Some(log_id(1, 1, 20)));

        let expected_time = {
            let mut queue = q.queue.lock().unwrap();
            let expected_time = queue[0].sending_time + std::time::Duration::from_secs(1);
            queue[1].sending_time = expected_time;
            expected_time
        };

        // Both requests ending at 10 are removed; the request ending at 20 remains.
        assert_eq!(q.drain_acked(&Some(log_id(1, 1, 10))), Some(expected_time));
        assert_eq!(q.queue.lock().unwrap().len(), 1);
        assert_eq!(q.queue.lock().unwrap()[0].last_log_id, Some(log_id(1, 1, 20)));
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
