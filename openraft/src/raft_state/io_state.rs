use std::error::Error;
use std::fmt;

use validit::Valid;
use validit::Validate;
use validit::less_equal;

use crate::LogId;
use crate::RaftTypeConfig;
use crate::raft_state::IOId;
use crate::raft_state::io_state::io_progress::IOProgress;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::VoteOf;

pub(crate) mod io_id;
pub(crate) mod io_progress;
pub(crate) mod log_io_id;

/// Tracks the state of completed I/O operations: log flushing, applying to state machine, and
/// snapshot building.
///
/// These states update only when I/O completes and may lag behind
/// [`RaftState`](`crate::RaftState`).
///
/// ## Progress Tracking
///
/// The log ids that are tracked include:
///
/// ```text
/// | log ids
/// | *------+-------+------+----+---+---+---+-------+------------------>
/// |        |       |      |    |   |   |   |       `---> log.accepted
/// |        |       |      |    |   |   |   `-----------> log.submitted
/// |        |       |      |    |   `-------------------> log.flushed
/// |        |       |      |    |       |
/// |        |       |      |    |       `---------------> apply.accepted
/// |        |       |      |    `-----------------------> apply.submitted
/// |        |       |      `----------------------------> apply.flushed
/// |        |       |
/// |        |       `-----------------------------------> snapshot
/// |        `-------------------------------------------> purged
/// ```
///
/// Each progress tracks three stages:
/// - **accepted**: Operation accepted but not yet submitted to I/O
/// - **submitted**: Submitted to I/O subsystem but not yet completed
/// - **flushed**: Successfully completed and persisted
///
/// **Note**: `apply.accepted` does not require `log.flushed`. A log only needs to be submitted
/// (not flushed) to be applied, since `RaftLogStorage` can read submitted entries.
///
/// For comprehensive details, see: [Log I/O Progress](crate::docs::data::log_io_progress).
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

    /// Tracks log I/O progress to local storage (vote + log entries).
    ///
    /// Uses `IOProgress<IOId>` to track both non-committed vote I/O and log I/O.
    /// See: [`IOId`](crate::docs::data::io_id)
    pub(crate) log_progress: Valid<IOProgress<IOId<C>>>,

    /// Tracks applying committed logs to state machine.
    ///
    /// - `accepted`: The committed log id, safe to apply
    /// - `submitted`: Sent to state machine task to apply
    /// - `flushed`: Already applied to state machine
    ///
    /// Uses `IOProgress<LogId>` since only committed logs are applied.
    pub(crate) apply_progress: Valid<IOProgress<LogId<C>>>,

    /// Tracks snapshot persistence progress.
    ///
    /// - `accepted`: Snapshot covering this log id should exist
    /// - `submitted`: Snapshot submitted to persist
    /// - `flushed`: Snapshot successfully persisted
    ///
    /// Tracks both locally built snapshots and snapshots installed from the leader.
    pub(crate) snapshot: Valid<IOProgress<LogId<C>>>,

    /// Last log id purged from storage.
    ///
    /// Unlike `RaftState::last_purged_log_id()` (which is the queued purge target),
    /// this reflects the actually purged log id.
    pub(crate) purged: Option<LogIdOf<C>>,
}

const LOG_PROGRESS_NAME: &str = "LogIO";
const APPLY_PROGRESS_NAME: &str = "Apply";
const SNAPSHOT_PROGRESS_NAME: &str = "Snapshot";

impl<C> Default for IOState<C>
where C: RaftTypeConfig
{
    fn default() -> Self {
        Self {
            building_snapshot: false,
            log_progress: new_progress(None, "xx", LOG_PROGRESS_NAME, false),
            apply_progress: new_progress(None, "xx", APPLY_PROGRESS_NAME, false),
            snapshot: new_progress(None, "xx", SNAPSHOT_PROGRESS_NAME, false),
            purged: None,
        }
    }
}

impl<C> Validate for IOState<C>
where C: RaftTypeConfig
{
    fn validate(&self) -> Result<(), Box<dyn Error>> {
        self.log_progress.validate()?;
        self.apply_progress.validate()?;

        // Disable this check, because IOId.log_id is None when a Vote request is just accepted(updated to
        // non-None when appendEntries are received):
        //
        // less_equal!(
        //     self.apply_progress.submitted(),
        //     self.log_progress.submitted().and_then(|x| x.last_log_id())
        // );

        self.snapshot.validate()?;
        // Snapshot must be included in applied.
        less_equal!(self.snapshot.submitted(), self.apply_progress.submitted());

        less_equal!(&self.purged, &self.snapshot.flushed().cloned());
        Ok(())
    }
}

impl<C> IOState<C>
where C: RaftTypeConfig
{
    /// Creates a new `IOState` with initial values.
    ///
    /// - `allow_io_notification_reorder`: Allow I/O completion notifications to arrive out of order
    pub(crate) fn new(
        id: &str,
        vote: &VoteOf<C>,
        applied: Option<LogIdOf<C>>,
        snapshot: Option<LogIdOf<C>>,
        purged: Option<LogIdOf<C>>,
        allow_io_notification_reorder: bool,
    ) -> Self {
        let reorder = allow_io_notification_reorder;

        Self {
            building_snapshot: false,
            log_progress: new_progress(Some(IOId::new(vote)), id, LOG_PROGRESS_NAME, reorder),
            apply_progress: new_progress(applied, id, APPLY_PROGRESS_NAME, reorder),
            snapshot: new_progress(snapshot, id, SNAPSHOT_PROGRESS_NAME, reorder),
            purged,
        }
    }

    pub(crate) fn applied(&self) -> Option<&LogIdOf<C>> {
        self.apply_progress.flushed()
    }

    pub(crate) fn snapshot(&self) -> Option<&LogIdOf<C>> {
        self.snapshot.flushed()
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

/// Creates a new `IOProgress` wrapped in `Valid`.
///
/// All three stages (accepted, submitted, flushed) are initialized to `initial_value`.
fn new_progress<T>(
    initial_value: Option<T>,
    id: impl ToString,
    name: &'static str,
    allow_notification_reorder: bool,
) -> Valid<IOProgress<T>>
where
    T: PartialOrd + fmt::Debug + fmt::Display + Clone,
{
    Valid::new(IOProgress::new_synchronized(
        initial_value,
        id,
        name,
        allow_notification_reorder,
    ))
}
