use std::borrow::Borrow;

use crate::summary::MessageSummary;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::NodeId;

#[derive(Clone, Copy, Debug)]
#[derive(PartialEq, Eq)]
pub(crate) struct Searching {
    /// Logs being sent to the target following node.
    pub(crate) mid: u64,

    /// One plus the max log index on the following node that might match the leader log.
    pub(crate) end: u64,
}

/// State of replication to a target node.
#[derive(Clone, Copy, Debug)]
#[derive(PartialEq, Eq)]
pub(crate) struct ProgressEntry<NID: NodeId> {
    /// The id of the last matching log on the target following node.
    pub(crate) matching: Option<LogId<NID>>,

    /// The last matching log index has not yet been determined.
    pub(crate) searching: Option<Searching>,
}

impl<NID: NodeId> ProgressEntry<NID> {
    #[allow(dead_code)]
    pub(crate) fn new(matching: Option<LogId<NID>>) -> Self {
        Self {
            matching,
            searching: None,
        }
    }

    /// Create a progress entry that does not have any matching log id.
    ///
    /// It's going to initiate a binary search to find the minimal matching log id.
    pub(crate) fn empty(end: u64) -> Self {
        let searching = if end == 0 {
            None
        } else {
            Some(Searching {
                mid: Self::calc_mid(0, end),
                end,
            })
        };

        Self {
            matching: None,
            searching,
        }
    }

    pub(crate) fn update_matching(&mut self, matching: Option<LogId<NID>>) {
        tracing::debug!(
            self = debug(&self),
            matching = display(matching.summary()),
            "update_matching"
        );

        debug_assert!(matching >= self.matching);

        self.matching = matching;

        if let Some(s) = &mut self.searching {
            let next = matching.next_index();

            if next >= s.end {
                self.searching = None;
            } else {
                s.mid = Self::calc_mid(next, s.end);
            }
        }
    }

    #[allow(dead_code)]
    pub(crate) fn update_conflicting(&mut self, conflict: u64) {
        tracing::debug!(self = debug(&self), conflict = display(conflict), "update_conflict");

        let matching_next = self.matching.next_index();

        debug_assert!(conflict >= matching_next);

        if let Some(s) = &mut self.searching {
            debug_assert!(conflict < s.end);
            debug_assert!(conflict >= s.mid);

            s.end = conflict;

            if matching_next >= s.end {
                self.searching = None;
            } else {
                s.mid = Self::calc_mid(matching_next, s.end);
            }
        } else {
            unreachable!("found conflict({}) when searching is None", conflict);
        }
    }

    /// Return the starting log index range(`[start,end)`) for the next AppendEntries.
    #[allow(dead_code)]
    pub(crate) fn sending_start(&self) -> (u64, u64) {
        match self.searching {
            None => {
                let next = self.matching.next_index();
                (next, next)
            }
            Some(s) => (s.mid, s.end),
        }
    }

    pub(crate) fn max_possible_matching(&self) -> Option<u64> {
        if let Some(s) = &self.searching {
            if s.end == 0 {
                None
            } else {
                Some(s.end - 1)
            }
        } else {
            self.matching.index()
        }
    }

    fn calc_mid(matching_next: u64, end: u64) -> u64 {
        debug_assert!(matching_next < end);
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

#[cfg(test)]
mod tests {
    use std::borrow::Borrow;

    use crate::progress::entry::ProgressEntry;
    use crate::LeaderId;
    use crate::LogId;

    fn log_id(index: u64) -> LogId<u64> {
        LogId {
            leader_id: LeaderId { term: 1, node_id: 1 },
            index,
        }
    }

    #[test]
    fn test_update_matching() -> anyhow::Result<()> {
        let mut pe = ProgressEntry::empty(20);
        assert_eq!(&None, pe.borrow());
        assert_eq!((8, 20), pe.sending_start());

        pe.update_matching(None);
        assert_eq!(&None, pe.borrow());
        assert_eq!((8, 20), pe.sending_start());

        pe.update_matching(Some(log_id(0)));
        assert_eq!(&Some(log_id(0)), pe.borrow());
        assert_eq!((9, 20), pe.sending_start());

        pe.update_matching(Some(log_id(0)));
        assert_eq!(&Some(log_id(0)), pe.borrow());
        assert_eq!((9, 20), pe.sending_start());

        pe.update_matching(Some(log_id(1)));
        assert_eq!(&Some(log_id(1)), pe.borrow());
        assert_eq!((10, 20), pe.sending_start());

        pe.update_matching(Some(log_id(4)));
        assert_eq!(&Some(log_id(4)), pe.borrow());
        assert_eq!((5, 20), pe.sending_start());

        // All logs are matching
        pe.update_matching(Some(log_id(19)));
        assert_eq!(&Some(log_id(19)), pe.borrow());
        assert_eq!((20, 20), pe.sending_start());

        pe.update_matching(Some(log_id(20)));
        assert_eq!(&Some(log_id(20)), pe.borrow());
        assert_eq!((21, 21), pe.sending_start());
        Ok(())
    }

    #[test]
    fn test_update_conflict() -> anyhow::Result<()> {
        let mut pe = ProgressEntry::empty(20);

        pe.update_matching(Some(log_id(4)));
        assert_eq!(&Some(log_id(4)), pe.borrow());
        assert_eq!((5, 20), pe.sending_start());

        pe.update_conflicting(19);
        assert_eq!(&Some(log_id(4)), pe.borrow());
        assert_eq!((5, 19), pe.sending_start());

        pe.update_conflicting(5);
        assert_eq!(&Some(log_id(4)), pe.borrow());
        assert_eq!((5, 5), pe.sending_start());
        assert!(pe.searching.is_none());

        Ok(())
    }
}
