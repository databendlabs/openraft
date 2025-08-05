use std::error::Error;

use validit::less_equal;
use validit::Valid;
use validit::Validate;

use crate::display_ext::DisplayOption;
use crate::raft_state::io_state::io_progress::IOProgress;
use crate::raft_state::IOId;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::VoteOf;
use crate::LogId;
use crate::RaftTypeConfig;

pub(crate) mod io_id;
pub(crate) mod io_progress;
pub(crate) mod log_io_id;

/// IOState tracks the state of actually happened io including log flushed, applying log to state
/// machine or snapshot building.
///
/// These states are updated only when the I/O completes and thus may fall behind to the state
/// stored in [`RaftState`](`crate::RaftState`).
///
/// The log ids that are tracked include:
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
#[derive(PartialEq, Eq)]
pub(crate) struct IOState<C>
where C: RaftTypeConfig
{
    /// Whether it is building a snapshot
    building_snapshot: bool,

    /// Tracks the accepted, submitted and flushed log I/O to the local storage.
    ///
    /// Note that log I/O also includes the vote state, which is persisted alongside log entries.
    pub(crate) log_progress: Valid<IOProgress<IOId<C>>>,

    /// The io progress of applying log to state machine.
    ///
    /// - The `apply_progress.accepted()` log id is also the committed, i.e., persisted in a quorum
    ///   and can be chosen by the next Leader. A quorum is either a uniform quorum or a joint
    ///   quorum. This is the highest log id that is safe to apply to the state machine.
    ///
    /// - The `apply_progress.submitted()` is the last log id that has been sent to the state
    ///   machine task to apply.
    ///
    /// - The `apply_progress.flushed()` is the last log id that has been already applied to state
    ///   machine.
    ///
    /// Note that depending on the implementation of the state machine,
    /// the `flushed()` log id may not be persisted in storage (the state machine may periodically
    /// build a snapshot to persist the state).
    pub(crate) apply_progress: Valid<IOProgress<LogId<C>>>,

    /// The last log id in the currently persisted snapshot.
    pub(crate) snapshot: Option<LogIdOf<C>>,

    /// The last log id that has been purged from storage.
    ///
    /// `RaftState::last_purged_log_id()`
    /// is just the log id that is going to be purged, i.e., there is a `PurgeLog` command queued to
    /// be executed, and it may not be the actually purged log id.
    pub(crate) purged: Option<LogIdOf<C>>,
}

const LOG_PROGRESS_NAME: &str = "LogIO";
const APPLY_PROGRESS_NAME: &str = "Apply";

impl<C> Default for IOState<C>
where C: RaftTypeConfig
{
    fn default() -> Self {
        Self {
            building_snapshot: false,
            log_progress: Valid::new(IOProgress::new_synchronized(None, LOG_PROGRESS_NAME)),
            apply_progress: Valid::new(IOProgress::new_synchronized(None, APPLY_PROGRESS_NAME)),
            snapshot: None,
            purged: None,
        }
    }
}

impl<C> Validate for IOState<C>
where C: RaftTypeConfig
{
    fn validate(&self) -> Result<(), Box<dyn Error>> {
        self.log_progress.validate()?;

        // TODO: enable this when get_initial_state() initialize the log io progress correctly
        // let a = &self.append_log;
        // Applied does not have to be flushed in local store.
        // less_equal!(self.applied.as_ref(), a.submitted().and_then(|x| x.last_log_id()));

        less_equal!(self.snapshot.as_ref(), self.applied());
        less_equal!(&self.purged, &self.snapshot);
        Ok(())
    }
}

impl<C> IOState<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(
        vote: &VoteOf<C>,
        applied: Option<LogIdOf<C>>,
        snapshot: Option<LogIdOf<C>>,
        purged: Option<LogIdOf<C>>,
    ) -> Self {
        Self {
            building_snapshot: false,
            log_progress: Valid::new(IOProgress::new_synchronized(Some(IOId::new(vote)), LOG_PROGRESS_NAME)),
            apply_progress: Valid::new(IOProgress::new_synchronized(applied, APPLY_PROGRESS_NAME)),
            snapshot,
            purged,
        }
    }

    pub(crate) fn applied(&self) -> Option<&LogIdOf<C>> {
        self.apply_progress.flushed()
    }

    pub(crate) fn update_snapshot(&mut self, log_id: Option<LogIdOf<C>>) {
        tracing::debug!(snapshot = display(DisplayOption(&log_id)), "{}", func_name!());

        debug_assert!(
            log_id >= self.snapshot,
            "snapshot log id should be monotonically increasing: current: {:?}, update: {:?}",
            self.snapshot,
            log_id
        );

        self.snapshot = log_id;
    }

    pub(crate) fn snapshot(&self) -> Option<&LogIdOf<C>> {
        self.snapshot.as_ref()
    }

    pub(crate) fn set_building_snapshot(&mut self, building: bool) {
        self.building_snapshot = building;
    }

    pub(crate) fn building_snapshot(&self) -> bool {
        self.building_snapshot
    }

    pub(crate) fn update_purged(&mut self, log_id: Option<LogIdOf<C>>) {
        self.purged = log_id;
    }

    pub(crate) fn purged(&self) -> Option<&LogIdOf<C>> {
        self.purged.as_ref()
    }
}
