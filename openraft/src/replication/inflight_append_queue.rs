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
/// When a response arrives with a matching log id, requests are drained through the first
/// request containing that log id, and its sending time is returned for RTT calculation.
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

    /// Removes requests through the first one with `prev_log_id <= matching <= last_log_id`
    /// and returns the sending time of the last removed request.
    ///
    /// For a conflict response, pass the rejected request's `prev_log_id` as `matching`.
    ///
    /// A partial success consumes its request too: the remaining logs will be sent in a new
    /// stream. Stop at the matched request so later empty requests with the same `last_log_id`
    /// remain queued for their own responses.
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
            if matching < &first.prev_log_id {
                break;
            }

            last = Some(first.sending_time);
            let reached_matching = matching <= &first.last_log_id;
            q.pop_front();

            if reached_matching {
                break;
            }
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
    fn test_drain_acked_partial_none_matching() {
        let q = InflightAppendQueue::<UTConfig>::new();
        q.push(None, Some(log_id(1, 1, 5)));
        q.push(Some(log_id(1, 1, 5)), Some(log_id(1, 1, 10)));

        let expected_time = q.queue.lock().unwrap()[0].sending_time;

        // A partial success may accept no entries and only confirm prev_log_id=None.
        assert_eq!(q.drain_acked(&None), Some(expected_time));
        assert_eq!(q.queue.lock().unwrap().len(), 1);
        assert_eq!(q.drain_acked(&None), None);
    }

    #[test]
    fn test_drain_acked_inside_request() {
        let q = InflightAppendQueue::<UTConfig>::new();
        q.push(Some(log_id(1, 1, 5)), Some(log_id(1, 1, 10)));
        q.push(Some(log_id(1, 1, 10)), Some(log_id(1, 1, 20)));

        let expected_time = q.queue.lock().unwrap()[0].sending_time;

        assert_eq!(q.drain_acked(&Some(log_id(1, 1, 7))), Some(expected_time));
        assert_eq!(q.queue.lock().unwrap().len(), 1);
        assert_eq!(q.queue.lock().unwrap()[0].last_log_id, Some(log_id(1, 1, 20)));
    }

    #[test]
    fn test_drain_acked_same_matching_for_distinct_requests() {
        let q = InflightAppendQueue::<UTConfig>::new();
        q.push(None, Some(log_id(1, 1, 10)));
        q.push(Some(log_id(1, 1, 10)), Some(log_id(1, 1, 10)));
        q.push(Some(log_id(1, 1, 10)), Some(log_id(1, 1, 20)));

        let first_time = {
            let mut queue = q.queue.lock().unwrap();
            let first_time = queue[0].sending_time;
            queue[1].sending_time = first_time + std::time::Duration::from_secs(1);
            queue[2].sending_time = first_time + std::time::Duration::from_secs(2);
            first_time
        };

        // Data success, empty request success, then a partial success accepting no new entries.
        for i in 0..3 {
            assert_eq!(
                q.drain_acked(&Some(log_id(1, 1, 10))),
                Some(first_time + std::time::Duration::from_secs(i))
            );
        }
        assert!(q.queue.lock().unwrap().is_empty());
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
