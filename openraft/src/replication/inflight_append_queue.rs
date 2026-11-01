use std::collections::VecDeque;

use display_more::DisplayOptionExt;

use crate::RaftTypeConfig;
use crate::replication::inflight_append::InflightAppend;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::LogIdOf;

/// A queue tracking in-flight AppendEntries requests for measuring replication latency.
///
/// Fully acknowledged requests are removed from the pending queue. The last
/// popped log boundary retains its earliest sending time, so delayed responses
/// cannot credit newer requests ending at the same log id.
pub(crate) struct InflightAppendQueue<C>
where C: RaftTypeConfig
{
    queue: VecDeque<InflightAppend<C>>,

    /// Earliest request at the last fully acknowledged log boundary.
    last_popped: Option<InflightAppend<C>>,
}

impl<C> InflightAppendQueue<C>
where C: RaftTypeConfig
{
    pub(crate) fn new() -> Self {
        Self {
            queue: VecDeque::with_capacity(32),
            last_popped: None,
        }
    }

    /// Records a new in-flight AppendEntries request.
    pub(crate) fn push(&mut self, prev_log_id: Option<LogIdOf<C>>, last_log_id: Option<LogIdOf<C>>) {
        let inflight = InflightAppend::new(prev_log_id, last_log_id);

        tracing::debug!("Inflight queue push: {}", inflight);

        self.queue.push_back(inflight)
    }

    /// Removes requests through the first range covering a conflict point and
    /// returns its conservative sending time.
    ///
    /// At a shared or retained boundary, the earlier request's sending time is used.
    /// Returns `None` for a conflict behind the retained boundary or outside all recorded ranges.
    pub(crate) fn drain_conflicted(&mut self, conflict_log_id: &LogIdOf<C>) -> Option<InstantOf<C>> {
        if let Some(previous) = &self.last_popped {
            let older_boundary = Some(conflict_log_id) < previous.last_log_id.as_ref();
            if older_boundary {
                return None;
            }
            let same_boundary = previous.last_log_id.as_ref() == Some(conflict_log_id);
            if same_boundary {
                return Some(previous.sending_time);
            }
        }

        while let Some(first) = self.queue.front() {
            let ends_before_conflict = first.last_log_id.as_ref() < Some(conflict_log_id);
            if ends_before_conflict {
                self.queue.pop_front();
                continue;
            }

            let starts_at_or_before_conflict = first.prev_log_id.as_ref() <= Some(conflict_log_id);
            if starts_at_or_before_conflict {
                let sending_time = first.sending_time;
                self.queue.pop_front();
                return Some(sending_time);
            }

            break;
        }

        None
    }

    /// Returns a conservative sending time for the acknowledged log progress.
    ///
    /// Keeps the earliest sending time at the last popped boundary. Requests
    /// sharing that boundary are drained without replacing its timestamp.
    /// Partial acknowledgements use the front pending request's sending time.
    /// Returns `None` for progress behind the retained boundary or without a known fully or
    /// partially acknowledged request. Progress beyond all recorded requests defensively returns
    /// the retained time, although such a response violates the transport contract.
    pub(crate) fn drain_acked(&mut self, matching: &Option<LogIdOf<C>>) -> Option<InstantOf<C>> {
        tracing::debug!(
            "Inflight queue drain_acked: matching: {}; last_popped: {:?}; data: {:?}",
            matching.display(),
            self.last_popped,
            self.queue.as_slices()
        );

        while let Some(first) = self.queue.front() {
            if matching < &first.last_log_id {
                break;
            }

            let popped = self.queue.pop_front().unwrap();
            let advances = self.last_popped.as_ref().is_none_or(|previous| popped.last_log_id > previous.last_log_id);
            if advances {
                self.last_popped = Some(popped);
            }
        }

        let previous = self.last_popped.as_ref();
        if let Some(previous) = previous {
            if matching < &previous.last_log_id {
                return None;
            }
            if matching == &previous.last_log_id {
                return Some(previous.sending_time);
            }
        }

        if let Some(first) = self.queue.front() {
            let partially_acked = matching > &first.prev_log_id;
            if partially_acked {
                return Some(first.sending_time);
            }
        }

        let previous = previous?;
        Some(previous.sending_time)
    }

    /// Returns `true` if there are no pending requests.
    pub(crate) fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

#[cfg(test)]
mod inflight_append_queue_test;
