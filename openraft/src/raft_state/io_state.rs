use std::error::Error;

use validit::less_equal;
use validit::Valid;
use validit::Validate;

use crate::display_ext::DisplayOption;
use crate::raft_state::io_state::io_progress::IOProgress;
use crate::raft_state::IOId;
use crate::LogId;
use crate::RaftTypeConfig;
use crate::Vote;

pub(crate) mod io_id;
pub(crate) mod io_progress;
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
/// | *------+-------+-------+-------+-------+-------+------------------>
/// |        |       |       |       |       |       `---> accepted
/// |        |       |       |       |       `-----------> submitted
/// |        |       |       |       `-------------------> flushed
/// |        |       |       `---------------------------> applied
/// |        |       `-----------------------------------> snapshot
/// |        `-------------------------------------------> purged
/// ```
///
/// - `accepted`: Accepted log entries from the Leader but not yet submit to the storage.
/// - `submitted`: AppendEntries IO request is submitted to `RaftLogStorage`, but not yet flushed.
/// - `flushed`: The log entries are persisted in the `RaftLogStorage`.
/// - `applied`: log entries are applied to state machine.
/// - `snapshot`: log entries are included in a persisted snapshot.
/// - `purged`: log entries are purged from `RaftLogStorage`.
///
/// Invariants:
///
/// ```text
///                                RaftLogStorage
/// .----------------------------------------------------------------------------.
/// | purged ≤ -.                             flushed ≤ -+- submitted ≤ accepted |
/// '-----------|----------------------------------------|-----------------------'
///             |                                        |
///             |                        .- committed ≤ -'
///             |                        |
///           .-|------------------------|-.
///           | '- snapshot ≤ applied ≤ -' |
///           '----------------------------'
///                  RaftStateMachine
/// ```
#[derive(Debug, Clone)]
#[derive(Default)]
#[derive(PartialEq, Eq)]
pub(crate) struct IOState<C>
where C: RaftTypeConfig
{
    /// Whether it is building a snapshot
    building_snapshot: bool,

    /// Tracks the accepted, submitted and flushed IO to local storage.
    pub(crate) io_progress: Valid<IOProgress<IOId<C>>>,

    /// The last log id that has been applied to state machine.
    pub(crate) applied: Option<LogId<C::NodeId>>,

    /// The last log id in the currently persisted snapshot.
    pub(crate) snapshot: Option<LogId<C::NodeId>>,

    /// The last log id that has been purged from storage.
    ///
    /// `RaftState::last_purged_log_id()`
    /// is just the log id that is going to be purged, i.e., there is a `PurgeLog` command queued to
    /// be executed, and it may not be the actually purged log id.
    pub(crate) purged: Option<LogId<C::NodeId>>,
}

impl<C> Validate for IOState<C>
where C: RaftTypeConfig
{
    fn validate(&self) -> Result<(), Box<dyn Error>> {
        self.io_progress.validate()?;

        // TODO: enable this when get_initial_state() initialize the log io progress correctly
        // let a = &self.append_log;
        // Applied does not have to be flushed in local store.
        // less_equal!(self.applied.as_ref(), a.submitted().and_then(|x| x.last_log_id()));

        less_equal!(&self.snapshot, &self.applied);
        less_equal!(&self.purged, &self.snapshot);
        Ok(())
    }
}

impl<C> IOState<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(
        vote: &Vote<C>,
        applied: Option<LogId<C::NodeId>>,
        snapshot: Option<LogId<C::NodeId>>,
        purged: Option<LogId<C::NodeId>>,
    ) -> Self {
        let mut io_progress = Valid::new(IOProgress::default());

        io_progress.accept(IOId::new(vote));
        io_progress.submit(IOId::new(vote));
        io_progress.flush(IOId::new(vote));

        Self {
            building_snapshot: false,
            io_progress,
            applied,
            snapshot,
            purged,
        }
    }

    pub(crate) fn update_applied(&mut self, log_id: Option<LogId<C::NodeId>>) {
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

    pub(crate) fn applied(&self) -> Option<&LogId<C::NodeId>> {
        self.applied.as_ref()
    }

    pub(crate) fn update_snapshot(&mut self, log_id: Option<LogId<C::NodeId>>) {
        tracing::debug!(snapshot = display(DisplayOption(&log_id)), "{}", func_name!());

        debug_assert!(
            log_id >= self.snapshot,
            "snapshot log id should be monotonically increasing: current: {:?}, update: {:?}",
            self.snapshot,
            log_id
        );

        self.snapshot = log_id;
    }

    pub(crate) fn snapshot(&self) -> Option<&LogId<C::NodeId>> {
        self.snapshot.as_ref()
    }

    pub(crate) fn set_building_snapshot(&mut self, building: bool) {
        self.building_snapshot = building;
    }

    pub(crate) fn building_snapshot(&self) -> bool {
        self.building_snapshot
    }

    pub(crate) fn update_purged(&mut self, log_id: Option<LogId<C::NodeId>>) {
        self.purged = log_id;
    }

    pub(crate) fn purged(&self) -> Option<&LogId<C::NodeId>> {
        self.purged.as_ref()
    }
}
