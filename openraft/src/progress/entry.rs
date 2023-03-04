use std::borrow::Borrow;
use std::error::Error;
use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Formatter;
use std::ops::Deref;

use crate::less;
use crate::less_equal;
use crate::progress::inflight::Inflight;
use crate::raft_state::LogStateReader;
use crate::summary::MessageSummary;
use crate::validate::Valid;
use crate::validate::Validate;
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
            matching,
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

    /// Return if a log id is inflight sending.
    ///
    /// `prev_log_id` is never inflight.
    pub(crate) fn is_inflight(&self, log_id: &LogId<NID>) -> bool {
        match &self.inflight {
            Inflight::None => false,
            Inflight::Logs { log_id_range, .. } => {
                let lid = Some(*log_id);
                lid > log_id_range.prev_log_id && lid <= log_id_range.last_log_id
            }
            Inflight::Snapshot { last_log_id: _, .. } => false,
        }
    }

    pub(crate) fn update_matching(&mut self, matching: Option<LogId<NID>>) {
        tracing::debug!(
            self = display(&self),
            matching = display(matching.summary()),
            "update_matching"
        );

        debug_assert!(matching >= self.matching);

        self.matching = matching;
        self.inflight.ack(self.matching);

        let matching_next = self.matching.next_index();
        self.searching_end = std::cmp::max(self.searching_end, matching_next);
    }

    #[allow(dead_code)]
    pub(crate) fn update_conflicting(&mut self, conflict: u64) {
        tracing::debug!(self = debug(&self), conflict = display(conflict), "update_conflict");

        self.inflight.conflict(conflict);

        debug_assert!(conflict < self.searching_end);
        self.searching_end = conflict;
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
        let purge_upto = log_state.purge_upto();
        let snapshot_last = log_state.snapshot_last_log_id();

        let last_next = log_state.last_log_id().next_index();
        let purge_upto_next = purge_upto.next_index();

        debug_assert!(
            self.searching_end <= last_next,
            "expect: searching_end: {} <= last_log_id.next_index: {}",
            self.searching_end,
            last_next
        );

        // `searching_end` is the max value for `start`.

        // The log the follower needs is purged.
        // Replicate by snapshot.
        if self.searching_end < purge_upto_next {
            self.curr_inflight_id += 1;
            self.inflight = Inflight::snapshot(snapshot_last.copied()).with_id(self.curr_inflight_id);
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

impl<NID: NodeId> Borrow<Option<LogId<NID>>> for Valid<ProgressEntry<NID>> {
    fn borrow(&self) -> &Option<LogId<NID>> {
        self.deref().borrow()
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
        less_equal!(self.matching.next_index(), self.searching_end);

        self.inflight.validate()?;

        match self.inflight {
            Inflight::None => {}
            Inflight::Logs { log_id_range, .. } => {
                // matching <= prev_log_id              <= last_log_id
                //             prev_log_id.next_index() <= searching_end
                less_equal!(self.matching, log_id_range.prev_log_id);
                less_equal!(log_id_range.prev_log_id.next_index(), self.searching_end);
            }
            Inflight::Snapshot { last_log_id, .. } => {
                // There is no need to send a snapshot smaller than last matching.
                less!(self.matching, last_log_id);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Borrow;

    use crate::progress::entry::ProgressEntry;
    use crate::progress::inflight::Inflight;
    use crate::raft_state::LogStateReader;
    use crate::CommittedLeaderId;
    use crate::LogId;

    fn log_id(index: u64) -> LogId<u64> {
        LogId {
            leader_id: CommittedLeaderId::new(1, 1),
            index,
        }
    }

    fn inflight_logs(prev_index: u64, last_index: u64) -> Inflight<u64> {
        Inflight::logs(Some(log_id(prev_index)), Some(log_id(last_index)))
    }
    #[test]
    fn test_is_inflight() -> anyhow::Result<()> {
        let mut pe = ProgressEntry::empty(20);
        assert_eq!(false, pe.is_inflight(&log_id(2)));

        pe.inflight = inflight_logs(2, 4);
        assert_eq!(false, pe.is_inflight(&log_id(1)));
        assert_eq!(false, pe.is_inflight(&log_id(2)));
        assert_eq!(true, pe.is_inflight(&log_id(3)));
        assert_eq!(true, pe.is_inflight(&log_id(4)));
        assert_eq!(false, pe.is_inflight(&log_id(5)));

        pe.inflight = Inflight::snapshot(Some(log_id(5)));
        assert_eq!(false, pe.is_inflight(&log_id(5)));

        Ok(())
    }

    #[test]
    fn test_update_matching() -> anyhow::Result<()> {
        // Update matching and inflight
        {
            let mut pe = ProgressEntry::empty(20);
            pe.inflight = inflight_logs(5, 10);
            pe.update_matching(Some(log_id(6)));
            assert_eq!(inflight_logs(6, 10), pe.inflight);
            assert_eq!(Some(log_id(6)), pe.matching);
            assert_eq!(20, pe.searching_end);

            pe.update_matching(Some(log_id(10)));
            assert_eq!(Inflight::None, pe.inflight);
            assert_eq!(Some(log_id(10)), pe.matching);
            assert_eq!(20, pe.searching_end);
        }

        // `searching_end` should be updated
        {
            let mut pe = ProgressEntry::empty(20);
            pe.matching = Some(log_id(6));
            pe.inflight = inflight_logs(5, 20);

            pe.update_matching(Some(log_id(20)));
            assert_eq!(21, pe.searching_end);
        }

        Ok(())
    }

    #[test]
    fn test_update_conflicting() -> anyhow::Result<()> {
        let mut pe = ProgressEntry::empty(20);
        pe.matching = Some(log_id(3));
        pe.inflight = inflight_logs(5, 10);
        pe.update_conflicting(5);
        assert_eq!(Inflight::None, pe.inflight);
        assert_eq!(&Some(log_id(3)), pe.borrow());
        assert_eq!(5, pe.searching_end);

        Ok(())
    }

    /// LogStateReader impl for testing
    struct LogState {
        last: Option<LogId<u64>>,
        snap_last: Option<LogId<u64>>,
        purge_upto: Option<LogId<u64>>,
        purged: Option<LogId<u64>>,
    }

    impl LogState {
        fn new(purge_upto: u64, snap_last: u64, last: u64) -> Self {
            Self {
                last: Some(log_id(last)),
                snap_last: Some(log_id(snap_last)),
                // `next_send()` only checks purge_upto, but not purged,
                // We just fake a purged
                purge_upto: Some(log_id(purge_upto)),
                purged: Some(log_id(purge_upto - 1)),
            }
        }
    }

    impl LogStateReader<u64> for LogState {
        fn get_log_id(&self, index: u64) -> Option<LogId<u64>> {
            let x = Some(log_id(index));
            if x >= self.purged && x <= self.last {
                x
            } else {
                None
            }
        }

        fn last_log_id(&self) -> Option<&LogId<u64>> {
            self.last.as_ref()
        }

        fn committed(&self) -> Option<&LogId<u64>> {
            unimplemented!("testing")
        }

        fn snapshot_last_log_id(&self) -> Option<&LogId<u64>> {
            self.snap_last.as_ref()
        }

        fn purge_upto(&self) -> Option<&LogId<u64>> {
            self.purge_upto.as_ref()
        }

        fn last_purged_log_id(&self) -> Option<&LogId<u64>> {
            self.purged.as_ref()
        }
    }

    #[test]
    fn test_next_send() -> anyhow::Result<()> {
        // There is already inflight data, return it in an Err
        {
            let mut pe = ProgressEntry::empty(20);
            pe.inflight = inflight_logs(10, 11);
            let res = pe.next_send(&LogState::new(6, 10, 20), 100);
            assert_eq!(Err(&inflight_logs(10, 11)), res);
        }

        {
            //    matching,end
            //    4,5
            //    v
            // -----+------+-----+--->
            //      purged snap  last
            //      6      10    20

            let mut pe = ProgressEntry::empty(4);
            pe.matching = Some(log_id(4));

            let res = pe.next_send(&LogState::new(6, 10, 20), 100);
            assert_eq!(Ok(&Inflight::snapshot(Some(log_id(10))).with_id(1)), res);
        }
        {
            //    matching,end
            //    4 6
            //    v-v
            // -----+------+-----+--->
            //      purged snap  last
            //      6      10    20

            let mut pe = ProgressEntry::empty(6);
            pe.matching = Some(log_id(4));

            let res = pe.next_send(&LogState::new(6, 10, 20), 100);
            assert_eq!(Ok(&Inflight::snapshot(Some(log_id(10))).with_id(1)), res);
        }

        {
            //    matching,end
            //    4  7
            //    v--v
            // -----+------+-----+--->
            //      purged snap  last
            //      6      10    20

            let mut pe = ProgressEntry::empty(7);
            pe.matching = Some(log_id(4));

            let res = pe.next_send(&LogState::new(6, 10, 20), 100);
            assert_eq!(Ok(&inflight_logs(6, 20).with_id(1)), res);
        }

        {
            //    matching,end
            //    4              20
            //    v--------------v
            // -----+------+-----+--->
            //      purged snap  last
            //      6      10    20

            let mut pe = ProgressEntry::empty(20);
            pe.matching = Some(log_id(4));

            let res = pe.next_send(&LogState::new(6, 10, 20), 100);
            assert_eq!(Ok(&inflight_logs(6, 20).with_id(1)), res);
        }

        //-----------

        {
            //      matching,end
            //      6,7
            //      v
            // -----+------+-----+--->
            //      purged snap  last
            //      6      10    20

            let mut pe = ProgressEntry::empty(7);
            pe.matching = Some(log_id(6));

            let res = pe.next_send(&LogState::new(6, 10, 20), 100);
            assert_eq!(Ok(&inflight_logs(6, 20).with_id(1)), res);
        }

        {
            //      matching,end
            //      6, 8
            //      v--v
            // -----+------+-----+--->
            //      purged snap  last
            //      6      10    20

            let mut pe = ProgressEntry::empty(8);
            pe.matching = Some(log_id(6));

            let res = pe.next_send(&LogState::new(6, 10, 20), 100);
            assert_eq!(Ok(&inflight_logs(6, 20).with_id(1)), res);
        }

        {
            //      matching,end
            //      6,           20
            //      v------------v
            // -----+------+-----+--->
            //      purged snap  last
            //      6      10    20

            let mut pe = ProgressEntry::empty(20);
            pe.matching = Some(log_id(6));

            let res = pe.next_send(&LogState::new(6, 10, 20), 100);
            assert_eq!(Ok(&inflight_logs(6, 20).with_id(1)), res);
        }

        {
            //       matching,end
            //       7,          20
            //       v-----------v
            // -----+------+-----+--->
            //      purged snap  last
            //      6      10    20

            let mut pe = ProgressEntry::empty(20);
            pe.matching = Some(log_id(7));

            let res = pe.next_send(&LogState::new(6, 10, 20), 100);
            assert_eq!(Ok(&inflight_logs(7, 20).with_id(1)), res);
        }

        {
            //          matching,end
            //          7,8
            //          v
            // -----+------+-----+--->
            //      purged snap  last
            //      6      10    20

            let mut pe = ProgressEntry::empty(8);
            pe.matching = Some(log_id(7));

            let res = pe.next_send(&LogState::new(6, 10, 20), 100);
            assert_eq!(Ok(&inflight_logs(7, 20).with_id(1)), res);
        }

        {
            //                   matching,end
            //                   20,21
            //                   v
            // -----+------+-----+--->
            //      purged snap  last
            //      6      10    20

            let mut pe = ProgressEntry::empty(21);
            pe.matching = Some(log_id(20));

            let res = pe.next_send(&LogState::new(6, 10, 20), 100);
            assert_eq!(Err(&Inflight::None), res, "nothing to send");
        }

        // Test max_entries
        {
            //       matching,end
            //       7,          20
            //       v-----------v
            // -----+------+-----+--->
            //      purged snap  last
            //      6      10    20

            let mut pe = ProgressEntry::empty(20);
            pe.matching = Some(log_id(7));

            let res = pe.next_send(&LogState::new(6, 10, 20), 5);
            assert_eq!(Ok(&inflight_logs(7, 12).with_id(1)), res);
        }
        Ok(())
    }
}
