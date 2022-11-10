use std::borrow::Borrow;

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
    pub(crate) fn empty(end: u64) -> Self {
        Self {
            matching: None,
            searching: Some(Searching { mid: end / 2, end }),
        }
    }

    pub(crate) fn update_matching(&mut self, matching: Option<LogId<NID>>) {
        self.matching = matching;

        if let Some(s) = &self.searching {
            if matching.next_index() >= s.end {
                self.searching = None;
            } else {

                // TODO: update mid
            }
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
}

impl<NID: NodeId> Borrow<Option<LogId<NID>>> for ProgressEntry<NID> {
    fn borrow(&self) -> &Option<LogId<NID>> {
        &self.matching
    }
}
