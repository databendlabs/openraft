use std::error::Error;
use std::fmt;

use display_more::DisplayOptionExt;
use validit::Valid;
use validit::Validate;
use validit::less_equal;

use crate::RaftTypeConfig;
use crate::raft_state::IOId;
use crate::raft_state::io_state::io_progress::IOProgress;
use crate::raft_state::io_state::log_io_id::LogIOId;
use crate::raft_state::io_state::monotonic::MonotonicIncrease;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::VoteOf;

pub(crate) mod io_id;
pub(crate) mod io_progress;
pub(crate) mod log_io_id;
pub(crate) mod monotonic;

/// Tracks the state of completed I/O operations: log flushing, applying to state machine, and
/// snapshot building.
///
/// ## RaftState vs IOState
///
/// [`RaftState`] represents the Engine's logical view — what the system *should* look like after
/// all queued I/O completes. [`IOState`] represents the Runtime's physical view — what has
/// actually been persisted and applied. Between them lies an I/O pipeline:
///
/// ```text
///  Engine            Command Queue            Runtime / Storage
///  ──────            ─────────────            ─────────────────
///  RaftState  ────>  AppendEntries,   ────>  IOState
///  (logical)        SaveVote,                (physical)
///                   PurgeLog, ...
///
///  vote: L2         queued: SaveVote(L2)     log_progress.accepted: L2
///  log_ids: [1..5]  queued: Append([3..5])   log_progress.flushed:   [1..2]
///                                            (entries 3..5 not yet on disk)
/// ```
///
/// This gap means [`IOState`] may lag behind [`RaftState`]. Commands like
/// [`Command::Respond`] use [`Condition::IOFlushed`] to delay client responses until the
/// relevant I/O completes, ensuring linearizability.
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
    pub(crate) apply_progress: Valid<IOProgress<LogIdOf<C>>>,

    /// Tracks snapshot persistence progress.
    ///
    /// - `accepted`: Snapshot covering this log id should exist
    /// - `submitted`: Snapshot submitted to persist
    /// - `flushed`: Snapshot successfully persisted
    ///
    /// Tracks both locally built snapshots and snapshots installed from the leader.
    pub(crate) snapshot: Valid<IOProgress<LogIdOf<C>>>,

    /// The highest log id committed by the cluster (replicated to a quorum).
    ///
    /// This tracks the cluster-level commit, which may differ from the local committed log id
    /// (safe to apply to state machine) when a commit notification arrives before the append RPC
    /// that writes the committed entry.
    ///
    /// **Type**: `LogIOId = (CommittedLeaderId, LogId)`
    ///
    /// Storing the leader's vote with the committed log ID provides:
    /// - **Self-documentation**: Explicitly records which leader sent this commit notification
    /// - **Update safety**: Forces callers to provide the leader's vote, preventing accidental
    ///   updates
    /// - **Implementation flexibility**: While OpenRaft enforces vote-first synchronization (making
    ///   the vote in `cluster_committed` equal to `log_progress.accepted()` vote), storing it
    ///   separately allows for alternative synchronization approaches in theory
    ///
    /// **For detailed explanation** with timeline examples, see:
    /// [Cluster-Committed vs Local Committed](crate::docs::protocol::commit)
    ///
    /// ## Update Conditions
    ///
    /// This value is updated in two scenarios:
    ///
    /// 1. **Leader**: When a log entry is replicated to and confirmed by a quorum, the leader
    ///    updates this value immediately, even if the entry hasn't been submitted to local storage.
    ///
    /// 2. **Follower**: The follower must first accept the leader's vote before accepting the
    ///    committed log id (vote-first protocol). Upon receiving the leader's committed log id, the
    ///    follower updates this value, then computes its local committed using
    ///    [`calculate_local_committed()`](Self::calculate_local_committed).
    pub(crate) cluster_committed: MonotonicIncrease<LogIOId<C>>,

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
            log_progress: new_progress(None, "xx", LOG_PROGRESS_NAME),
            apply_progress: new_progress(None, "xx", APPLY_PROGRESS_NAME),
            snapshot: new_progress(None, "xx", SNAPSHOT_PROGRESS_NAME),
            cluster_committed: MonotonicIncrease::default(),
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
    pub(crate) fn new(
        id: &str,
        vote: &VoteOf<C>,
        applied: Option<LogIdOf<C>>,
        snapshot: Option<LogIdOf<C>>,
        purged: Option<LogIdOf<C>>,
    ) -> Self {
        Self {
            building_snapshot: false,
            log_progress: new_progress(Some(IOId::new(vote)), id, LOG_PROGRESS_NAME),
            apply_progress: new_progress(applied, id, APPLY_PROGRESS_NAME),
            snapshot: new_progress(snapshot, id, SNAPSHOT_PROGRESS_NAME),
            cluster_committed: MonotonicIncrease::default(),
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

    /// Calculates the local committed log id that is safe to apply to the state machine.
    ///
    /// Returns `min(cluster_committed.log_id, log_progress.accepted().log_id)` when the safety
    /// condition `accepted_vote >= cluster_committed_vote` holds.
    ///
    /// With the vote-first synchronization protocol, `accepted_vote` is always equal to
    /// `cluster_committed_vote` because the vote is synchronized before any commit notification.
    /// The safety check still exists in the code but always passes, serving as a defensive
    /// assertion.
    ///
    /// **For detailed explanation** with timeline examples, see:
    /// [Cluster-Committed vs Local Committed](crate::docs::protocol::commit)
    pub(crate) fn calculate_local_committed(&mut self) -> Option<LogIdOf<C>> {
        let local_committed = self.do_calculate_local_committed();

        tracing::debug!(
            "{}, cluster_committed: {}, accepted: {}, local_committed: {}",
            func_name!(),
            self.cluster_committed.value().display(),
            self.log_progress.accepted().display(),
            local_committed.display()
        );

        local_committed
    }

    pub(crate) fn do_calculate_local_committed(&mut self) -> Option<LogIdOf<C>> {
        let cluster_committed = self.cluster_committed.value()?.clone();
        let accepted = self.log_progress.accepted()?.clone();

        let cluster_committed_vote = cluster_committed.to_committed_vote();
        let accepted_vote = accepted.to_committed_vote()?;

        // If accepted_vote is smaller than the cluster_committed_vote,
        // There may be inflight RPC that might truncate a committed log entry(and then re-append it).
        // Thus, the state machine may not be able to read a log entry even it is smaller than the committed
        // log id.
        //
        // A committed log id will always be seen by future leader.
        // Thus, once accepted_vote is greater than the cluster_committed_vote, all enqueued IO won't
        // truncate any log entries smaller than the committed, thus it is safe to apply them to the
        // state machine.
        if accepted_vote >= cluster_committed_vote {
            std::cmp::min(
                accepted.last_log_id().cloned(),
                cluster_committed.last_log_id().cloned(),
            )
        } else {
            None
        }
    }

    /// Checks whether committed log entries can be safely applied to the state machine.
    ///
    /// This is the **leader safety invariant**: submitted and accepted I/O must originate from
    /// the same leader. When this holds, the submitted entries are guaranteed not to be
    /// truncated by any queued command, because a leader never truncates its own entries.
    ///
    /// If leaders differ, queued commands may override submitted logs:
    ///
    /// - submitted: append-entries(leader=L1, entry=E2)
    /// - queued: truncate(E2), save-vote(L2), append-entries(leader=L2, entry=E2')
    ///
    /// E2 will be overridden by E2' when the queue executes.
    pub(crate) fn can_safely_apply(&self) -> bool {
        let log_progress = &self.log_progress;
        log_progress.submitted().map(|x| x.as_ref_vote()) == log_progress.accepted().map(|x| x.as_ref_vote())
    }
}

/// Creates a new `IOProgress` wrapped in `Valid`.
///
/// All three stages (accepted, submitted, flushed) are initialized to `initial_value`.
fn new_progress<T>(initial_value: Option<T>, id: impl ToString, name: &'static str) -> Valid<IOProgress<T>>
where T: PartialOrd + fmt::Debug + fmt::Display + Clone {
    Valid::new(IOProgress::new_synchronized(initial_value, id, name))
}
