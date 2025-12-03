use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;

use crate::RaftTypeConfig;
use crate::display_ext::DisplayOptionExt;
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
    pub(crate) fn push(&self, log_id: Option<LogIdOf<C>>) {
        let mut q = self.queue.lock().unwrap();
        let inflight = InflightAppend::new(log_id);

        tracing::debug!("Inflight queue push: {}", inflight);

        q.push_back(inflight)
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
        loop {
            let Some(first) = q.front() else { break };
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
    fn test_push_and_drain_acked_none_matching() {
        let q = InflightAppendQueue::<UTConfig>::new();
        q.push(Some(log_id(1, 1, 5)));
        q.push(Some(log_id(1, 1, 10)));

        // matching=None is less than any log_id, so nothing is acked
        assert_eq!(q.drain_acked(&None), None);

        // Queue should remain unchanged
        assert_eq!(q.queue.lock().unwrap().len(), 2);
    }

    #[test]
    fn test_drain_acked_partial() {
        let q = InflightAppendQueue::<UTConfig>::new();
        q.push(Some(log_id(1, 1, 5)));
        q.push(Some(log_id(1, 1, 10)));
        q.push(Some(log_id(1, 1, 15)));

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
        q.push(Some(log_id(1, 1, 5)));
        q.push(Some(log_id(1, 1, 10)));

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
        q.push(None);
        q.push(Some(log_id(1, 1, 5)));

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
