use std::borrow::Borrow;
use std::error::Error;
use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Formatter;

use validit::Validate;

use crate::display_ext::DisplayOptionExt;
use crate::progress::inflight::Inflight;
use crate::progress::inflight::InflightError;
use crate::raft_state::LogStateReader;
use crate::summary::MessageSummary;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::NodeId;

/// State of replication to a target node.
#[derive(Clone, Copy, Debug)]
#[derive(PartialEq, Eq)]
pub(crate) struct ProgressEntry<NID: NodeId> {
    /// The id of the last matching log on the target following node.
    pub(crate) matching: Option<LogId<NID>>,

    pub(crate) curr_inflight_id: u64,

    /// The data being transmitted in flight.
    ///
    /// A non-none inflight expects a response when the data was successfully sent or failed.
    pub(crate) inflight: Inflight<NID>,

    /// One plus the max log index on the following node that might match the leader log.
    pub(crate) searching_end: u64,
}

impl<NID: NodeId> ProgressEntry<NID> {
    #[allow(dead_code)]
    pub(crate) fn new(matching: Option<LogId<NID>>) -> Self {
        Self {
            matching: matching.clone(),
            curr_inflight_id: 0,
            inflight: Inflight::None,
            searching_end: matching.next_index(),
        }
    }

    /// Create a progress entry that does not have any matching log id.
    ///
    /// It's going to initiate a binary search to find the minimal matching log id.
    pub(crate) fn empty(end: u64) -> Self {
        Self {
            matching: None,
            curr_inflight_id: 0,
            inflight: Inflight::None,
            searching_end: end,
        }
    }

    // This method is only used by tests.
    #[allow(dead_code)]
    pub(crate) fn with_curr_inflight_id(mut self, v: u64) -> Self {
        self.curr_inflight_id = v;
        self
    }

    // This method is only used by tests.
    #[allow(dead_code)]
    pub(crate) fn with_inflight(mut self, inflight: Inflight<NID>) -> Self {
        debug_assert_eq!(self.inflight, Inflight::None);

        self.inflight = inflight;
        self
    }

    /// Return if a range of log id `..=log_id` is inflight sending.
    ///
    /// `prev_log_id` is never inflight.
    pub(crate) fn is_log_range_inflight(&self, upto: &LogId<NID>) -> bool {
        match &self.inflight {
            Inflight::None => false,
            Inflight::Logs { log_id_range, .. } => {
                let lid = Some(upto.clone());
                lid > log_id_range.prev
            }
            Inflight::Snapshot { last_log_id: _, .. } => false,
        }
    }

    pub(crate) fn update_matching(
        &mut self,
        request_id: u64,
        matching: Option<LogId<NID>>,
    ) -> Result<(), InflightError> {
        tracing::debug!(
            self = display(&self),
            request_id = display(request_id),
            matching = display(matching.summary()),
            "update_matching"
        );

        self.inflight.ack(request_id, matching.clone())?;

        debug_assert!(matching >= self.matching);
        self.matching = matching;

        let matching_next = self.matching.next_index();
        self.searching_end = std::cmp::max(self.searching_end, matching_next);

        Ok(())
    }

    /// Update conflicting log index.
    ///
    /// Conflicting log index is the last found log index on a follower that is not matching the
    /// leader log.
    ///
    /// `request_id` is `None` for a Heartbeat request, meaning there is no inflight data, and
    /// `Some(request_id)` for a normal AppendEntries request.
    ///
    /// Usually if follower's data is lost, `conflict` is always greater than or equal `matching`.
    /// But for testing purpose, a follower is allowed to clean its data and wait for leader to
    /// replicate all data to it.
    ///
    /// To allow a follower to clean its data, enable feature flag [`loosen-follower-log-revert`] .
    ///
    /// [`loosen-follower-log-revert`]: crate::docs::feature_flags#feature-flag-loosen-follower-log-revert
    pub(crate) fn update_conflicting(&mut self, request_id: Option<u64>, conflict: u64) -> Result<(), InflightError> {
        tracing::debug!(
            "update_conflict: current_progress={}, request_id={}, conflict={}",
            self,
            request_id.display(),
            conflict
        );

        if let Some(request_id) = request_id {
            self.inflight.conflict(request_id, conflict)?;
        }

        if conflict >= self.searching_end {
            tracing::debug!(
                "conflict {} >= searching_end {}; no need to update",
                conflict,
                self.searching_end
            );
            return Ok(());
        }
        self.searching_end = conflict;

        // An already matching log id is found lost:
        //
        // - If log reversion is allowed, just restart the binary search from the beginning.
        // - Otherwise, panic it.
        {
            #[cfg(feature = "loosen-follower-log-revert")]
            if conflict < self.matching.next_index() {
                tracing::warn!(
                    "conflict {} < last matching {}: follower log is reverted; with 'loosen-follower-log-revert' enabled, this is allowed.",
                    conflict,
                    self.matching.display(),
                );

                self.matching = None;
            }

            debug_assert!(
                conflict >= self.matching.next_index(),
                "follower log reversion is not allowed \
                without `--features loosen-follower-log-revert`; \
                matching: {}; conflict: {}",
                self.matching.display(),
                conflict
            );
        }
        Ok(())
    }

    /// Initialize a replication action: sending log entries or sending snapshot.
    ///
    /// If there is an action in progress, i.e., `inflight` is not None, it returns an `Err`
    /// containing the current `inflight` data
    #[allow(dead_code)]
    pub(crate) fn next_send(
        &mut self,
        log_state: &impl LogStateReader<NID>,
        max_entries: u64,
    ) -> Result<&Inflight<NID>, &Inflight<NID>> {
        if !self.inflight.is_none() {
            return Err(&self.inflight);
        }

        let last_next = log_state.last_log_id().next_index();
        debug_assert!(
            self.searching_end <= last_next,
            "expect: searching_end: {} <= last_log_id.next_index: {}",
            self.searching_end,
            last_next
        );

        let purge_upto_next = {
            let purge_upto = log_state.purge_upto();
            purge_upto.next_index()
        };

        // `searching_end` is the max value for `start`.

        // The log the follower needs is purged.
        // Replicate by snapshot.
        if self.searching_end < purge_upto_next {
            self.curr_inflight_id += 1;
            let snapshot_last = log_state.snapshot_last_log_id();
            self.inflight = Inflight::snapshot(snapshot_last.cloned()).with_id(self.curr_inflight_id);
            return Ok(&self.inflight);
        }

        // Replicate by logs.
        // Run a binary search to find the matching log id, if matching log id is not determined.
        let mut start = Self::calc_mid(self.matching.next_index(), self.searching_end);
        if start < purge_upto_next {
            start = purge_upto_next;
        }

        let end = std::cmp::min(start + max_entries, last_next);

        if start == end {
            self.inflight = Inflight::None;
            return Err(&self.inflight);
        }

        let prev = log_state.prev_log_id(start);
        let last = log_state.prev_log_id(end);

        self.curr_inflight_id += 1;
        self.inflight = Inflight::logs(prev, last).with_id(self.curr_inflight_id);

        Ok(&self.inflight)
    }

    /// Return the index range(`[start,end]`) of the first log in the next AppendEntries.
    ///
    /// The returned range is left close and right close.
    #[allow(dead_code)]
    pub(crate) fn sending_start(&self) -> (u64, u64) {
        let mid = Self::calc_mid(self.matching.next_index(), self.searching_end);
        (mid, self.searching_end)
    }

    fn calc_mid(matching_next: u64, end: u64) -> u64 {
        debug_assert!(matching_next <= end);
        let d = end - matching_next;
        let offset = d / 16 * 8;
        matching_next + offset
    }
}

impl<NID: NodeId> Borrow<Option<LogId<NID>>> for ProgressEntry<NID> {
    fn borrow(&self) -> &Option<LogId<NID>> {
        &self.matching
    }
}

impl<NID: NodeId> Display for ProgressEntry<NID> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{{[{}, {}), inflight:{}}}",
            self.matching.summary(),
            self.searching_end,
            self.inflight
        )
    }
}

impl<NID: NodeId> Validate for ProgressEntry<NID> {
    fn validate(&self) -> Result<(), Box<dyn Error>> {
        validit::less_equal!(self.matching.next_index(), self.searching_end);

        self.inflight.validate()?;

        match &self.inflight {
            Inflight::None => {}
            Inflight::Logs { log_id_range, .. } => {
                // matching <= prev_log_id              <= last_log_id
                //             prev_log_id.next_index() <= searching_end
                validit::less_equal!(&self.matching, &log_id_range.prev);
                validit::less_equal!(log_id_range.prev.next_index(), self.searching_end);
            }
            Inflight::Snapshot { last_log_id, .. } => {
                // There is no need to send a snapshot smaller than last matching.
                validit::less!(&self.matching, &last_log_id);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
