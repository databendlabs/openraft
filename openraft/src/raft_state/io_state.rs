use log_io_id::LogIOId;

use crate::display_ext::DisplayOption;
use crate::LogId;
use crate::NodeId;
use crate::Vote;

pub(crate) mod log_io_id;

/// IOState tracks the state of actually happened io including log flushed, applying log to state
/// machine or snapshot building.
///
/// These states are updated only when the io complete and thus may fall behind to the state stored
/// in [`RaftState`](`crate::RaftState`),.
///
/// The log ids that are tracked includes:
///
/// ```text
/// | log ids
/// | *------------+---------+---------+---------+------------------>
/// |              |         |         |         `---> flushed
/// |              |         |         `-------------> applied
/// |              |         `-----------------------> snapshot
/// |              `---------------------------------> purged
/// ```
#[derive(Debug, Clone, Copy)]
#[derive(Default)]
#[derive(PartialEq, Eq)]
pub(crate) struct IOState<NID: NodeId> {
    /// Whether it is building a snapshot
    building_snapshot: bool,

    /// The last flushed vote.
    pub(crate) vote: Vote<NID>,

    /// The last log id that has been flushed to storage.
    // TODO: this wont be used until we move log io into separate task.
    pub(crate) flushed: LogIOId<NID>,

    /// The last log id that has been applied to state machine.
    pub(crate) applied: Option<LogId<NID>>,

    /// The last log id in the currently persisted snapshot.
    pub(crate) snapshot: Option<LogId<NID>>,

    /// The last log id that has been purged from storage.
    ///
    /// `RaftState::last_purged_log_id()`
    /// is just the log id that is going to be purged, i.e., there is a `PurgeLog` command queued to
    /// be executed, and it may not be the actually purged log id.
    pub(crate) purged: Option<LogId<NID>>,
}

impl<NID: NodeId> IOState<NID> {
    pub(crate) fn new(
        vote: Vote<NID>,
        flushed: LogIOId<NID>,
        applied: Option<LogId<NID>>,
        snapshot: Option<LogId<NID>>,
        purged: Option<LogId<NID>>,
    ) -> Self {
        Self {
            building_snapshot: false,
            vote,
            flushed,
            applied,
            snapshot,
            purged,
        }
    }

    pub(crate) fn update_vote(&mut self, vote: Vote<NID>) {
        self.vote = vote;
    }

    pub(crate) fn vote(&self) -> &Vote<NID> {
        &self.vote
    }

    pub(crate) fn update_applied(&mut self, log_id: Option<LogId<NID>>) {
        tracing::debug!(applied = display(DisplayOption(&log_id)), "{}", func_name!());

        // TODO: should we update flushed if applied is newer?
        debug_assert!(
            log_id > self.applied,
            "applied log id should be monotonically increasing: current: {:?}, update: {:?}",
            self.applied,
            log_id
        );

        self.applied = log_id;
    }

    pub(crate) fn applied(&self) -> Option<&LogId<NID>> {
        self.applied.as_ref()
    }

    pub(crate) fn update_snapshot(&mut self, log_id: Option<LogId<NID>>) {
        tracing::debug!(snapshot = display(DisplayOption(&log_id)), "{}", func_name!());

        // Two back-to-back `Raft::trigger().snapshot()` calls without any new log entry being
        // applied in between can legitimately reach this point with `log_id == self.snapshot`:
        // the first BuildSnapshot completes and clears `building_snapshot`, the second trigger
        // queues another build at the same `last_applied`, and when it finishes we get here
        // with an equal log id. `SnapshotHandler::update_snapshot` already returns early in
        // that case, so the io_state update is a no-op; only a strict regression is a real
        // invariant violation worth panicking on in debug.
        if log_id <= self.snapshot {
            debug_assert_eq!(
                log_id, self.snapshot,
                "snapshot log id must not regress: current: {:?}, update: {:?}",
                self.snapshot, log_id
            );
            return;
        }

        self.snapshot = log_id;
    }

    pub(crate) fn snapshot(&self) -> Option<&LogId<NID>> {
        self.snapshot.as_ref()
    }

    pub(crate) fn set_building_snapshot(&mut self, building: bool) {
        self.building_snapshot = building;
    }

    pub(crate) fn building_snapshot(&self) -> bool {
        self.building_snapshot
    }

    pub(crate) fn update_purged(&mut self, log_id: Option<LogId<NID>>) {
        self.purged = log_id;
    }

    pub(crate) fn purged(&self) -> Option<&LogId<NID>> {
        self.purged.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::IOState;
    use crate::testing::log_id;

    #[test]
    fn update_snapshot_accepts_equal_log_id() {
        // Two `Raft::trigger().snapshot()` calls with no new log entry applied in between
        // can legitimately cause `update_snapshot` to be invoked twice with the same log id.
        // This must not panic.
        let mut io_state = IOState::<u64>::default();

        io_state.update_snapshot(Some(log_id(1, 1, 5)));
        assert_eq!(Some(&log_id(1, 1, 5)), io_state.snapshot());

        // Second update with the same log id is a no-op, not a panic.
        io_state.update_snapshot(Some(log_id(1, 1, 5)));
        assert_eq!(Some(&log_id(1, 1, 5)), io_state.snapshot());
    }

    #[test]
    fn update_snapshot_advances_monotonically() {
        let mut io_state = IOState::<u64>::default();

        io_state.update_snapshot(Some(log_id(1, 1, 1)));
        assert_eq!(Some(&log_id(1, 1, 1)), io_state.snapshot());

        io_state.update_snapshot(Some(log_id(1, 1, 5)));
        assert_eq!(Some(&log_id(1, 1, 5)), io_state.snapshot());

        io_state.update_snapshot(Some(log_id(2, 1, 7)));
        assert_eq!(Some(&log_id(2, 1, 7)), io_state.snapshot());
    }

    #[test]
    #[should_panic(expected = "snapshot log id must not regress")]
    #[cfg(debug_assertions)]
    fn update_snapshot_rejects_regressing_log_id() {
        let mut io_state = IOState::<u64>::default();

        io_state.update_snapshot(Some(log_id(1, 1, 10)));
        // Going backward is a real invariant violation.
        io_state.update_snapshot(Some(log_id(1, 1, 5)));
    }
}
