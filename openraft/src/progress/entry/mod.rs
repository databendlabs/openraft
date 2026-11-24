pub(crate) mod update;

use std::error::Error;
use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Formatter;

use display_more::DisplayOptionExt;
use validit::Validate;

use crate::LogIdOptionExt;
use crate::RaftState;
use crate::RaftTypeConfig;
use crate::engine::EngineConfig;
use crate::progress::VecProgressEntry;
use crate::progress::VecProgressEntryData;
use crate::progress::entry::update::Updater;
use crate::progress::inflight::Inflight;
use crate::progress::stream_id::StreamId;
use crate::raft_state::LogStateReader;
use crate::type_config::alias::LogIdOf;

/// State of replication to a target node.
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
pub(crate) struct ProgressEntry<C>
where C: RaftTypeConfig
{
    pub(crate) id: C::NodeId,

    /// The id of the last matching log on the target following node.
    pub(crate) matching: Option<LogIdOf<C>>,

    pub(crate) data: ProgressData<C>,
}

/// Application-owned replication state that is not used for quorum calculation.
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
pub(crate) struct ProgressData<C>
where C: RaftTypeConfig
{
    pub(crate) stream_id: StreamId,

    /// The data being transmitted in flight.
    ///
    /// A non-none inflight expects a response when the data was successfully sent or failed.
    pub(crate) inflight: Inflight<C>,

    /// One plus the max log index on the following node that might match the leader log.
    pub(crate) searching_end: u64,

    /// Index of the first entry for the next probe to send, tried before binary search.
    ///
    /// The probe's `prev_log_id` is the entry before it. The next send consumes it, and skips it
    /// if it no longer lies in the search range.
    pub(crate) probe_start: Option<u64>,

    /// If true, reset the progress by setting matching to `None` when the follower's
    /// log is found reverted to an early state.
    ///
    /// This allows the target node to clean its data and wait for the leader to replicate all data
    /// to it.
    ///
    /// This flag will be cleared after the progress entry is reset.
    pub(crate) allow_log_reversion: bool,
}

impl<C> ProgressEntry<C>
where C: RaftTypeConfig
{
    #[cfg(test)]
    pub(crate) fn testing_new(id: C::NodeId, matching: Option<LogIdOf<C>>) -> Self {
        Self {
            id,
            matching: matching.clone(),
            data: ProgressData::new(StreamId::new(0), matching.next_index()),
        }
    }

    /// Create a progress entry that does not have any matching log id.
    ///
    /// It's going to initiate a binary search to find the minimal matching log id.
    pub(crate) fn empty(id: C::NodeId, stream_id: StreamId, end: u64) -> Self {
        Self {
            id,
            matching: None,
            data: ProgressData::new(stream_id, end),
        }
    }

    /// Start the next probe at entry `start`, before falling back to binary search.
    pub(crate) fn with_probe_start(mut self, start: u64) -> Self {
        self.data.probe_start = Some(start);
        self
    }

    pub(crate) fn matching(&self) -> Option<&LogIdOf<C>> {
        self.matching.as_ref()
    }

    // This method is only used by tests.
    #[cfg(test)]
    pub(crate) fn with_inflight(mut self, inflight: Inflight<C>) -> Self {
        debug_assert_eq!(self.data.inflight, Inflight::None);

        self.data.inflight = inflight;
        self
    }

    pub(crate) fn new_updater<'a>(&'a mut self, engine_config: &'a EngineConfig<C>) -> Updater<'a, C> {
        Updater::new(engine_config, self)
    }
}

impl<C> ProgressData<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(stream_id: StreamId, searching_end: u64) -> Self {
        Self {
            stream_id,
            inflight: Inflight::None,
            searching_end,
            probe_start: None,
            allow_log_reversion: false,
        }
    }
}

impl<C> VecProgressEntry for ProgressEntry<C>
where C: RaftTypeConfig
{
    type Id = C::NodeId;
    type Progress = Option<LogIdOf<C>>;

    fn id(&self) -> &Self::Id {
        &self.id
    }

    fn progress(&self) -> &Self::Progress {
        &self.matching
    }

    fn progress_mut(&mut self) -> &mut Self::Progress {
        &mut self.matching
    }
}

impl<C> VecProgressEntryData for ProgressEntry<C>
where C: RaftTypeConfig
{
    type Data = ProgressData<C>;

    fn data(&self) -> &Self::Data {
        &self.data
    }

    fn data_mut(&mut self) -> &mut Self::Data {
        &mut self.data
    }
}

impl<C> ProgressEntry<C>
where C: RaftTypeConfig
{
    /// Return if a range of log id `..=log_id` is inflight sending.
    ///
    /// `prev_log_id` is never inflight.
    pub(crate) fn is_log_range_inflight(&self, upto: &LogIdOf<C>) -> bool {
        match &self.data.inflight {
            Inflight::None => false,
            Inflight::Probe { log_id_range, .. } => {
                let lid = Some(upto);
                lid > log_id_range.prev.as_ref()
            }
            Inflight::Snapshot { inflight_id: _ } => false,
            Inflight::LogsSince { prev, .. } => {
                // All logs after prev are inflight in streaming mode
                let lid = Some(upto);
                lid > prev.as_ref()
            }
            Inflight::Clear { prev, .. } => {
                // The replication task may still read any log after prev for the dropped plan.
                let lid = Some(upto);
                lid > prev.as_ref()
            }
        }
    }

    /// Initialize a replication action: sending log entries or sending a snapshot.
    ///
    /// If there is an action in progress, i.e., `inflight` is not None, it returns an `Err`
    /// containing the current `inflight` data.
    ///
    /// See: [Algorithm to find the last matching log id on a Follower][algo].
    ///
    /// # Decision logic
    ///
    /// The follower's last log id matching the leader's log is known to lie in the range
    /// `[matching, searching_end)`; the invariant `matching.next_index() <= searching_end`
    /// always holds. This puts the progress in one of two regimes:
    ///
    /// - **Probing** (`matching.next_index() < searching_end`): the exact matching point is not yet
    ///   determined. Send one AppendEntries from the candidate range `(prev, last]`
    ///   ([`Inflight::Probe`]) with `prev` at a binary-search midpoint: a success response raises
    ///   `matching`, a conflict response lowers `searching_end`, until the range collapses.
    ///
    /// - **Pipeline** (`matching.next_index() == searching_end`): the matching point is exactly
    ///   `matching`. Stream all logs after it, with no fixed upper bound ([`Inflight::LogsSince`]).
    ///
    /// A pending [`ProgressData::probe_start`] is tried before either regime applies. It is raised
    /// to the purge boundary and used only if it then lies in
    /// `[matching.next_index(), searching_end]`. Every send consumes it.
    ///
    /// Purging constrains what AppendEntries can be built. `purge_upto` is the boundary of a
    /// scheduled purge, which may not have happened yet: the leader postpones it while a pending
    /// replication may still read entries at or below it, see
    /// [`ReplicationHandler::try_purge_log`]. A new payload never includes entries at index
    /// `<= purge_upto`, so that new sends cannot keep postponing the purge. The log id at
    /// `purge_upto` stays known: [`RaftState::log_ids`] holds it as a stored entry's id while the
    /// purge waits, and as the last purged log id afterwards. It does not come from the snapshot,
    /// which may end later, such as when the purge keeps the last
    /// [`Config::max_in_snapshot_log_to_keep`] logs of the snapshot. Thus the lowest usable `prev`
    /// is `purge_upto`. A snapshot must be sent instead of logs in exactly two situations:
    ///
    /// 1. `searching_end < purge_upto_next`, in either regime: every candidate matching position
    ///    lies strictly below the purge boundary. The lowest possible probe, `prev = purge_upto`,
    ///    sits at an index `>= searching_end` — a position already known not to match — so the
    ///    follower would reply with a conflict at that same index, which carries no new information
    ///    and is discarded (see [`Updater::update_conflicting`]): log replication cannot make
    ///    progress. `searching_end == purge_upto_next` is excluded: `prev = purge_upto` is then at
    ///    index `searching_end - 1`, still a candidate position worth probing.
    ///
    /// 2. Probing while the purge boundary covers the leader log (`purge_upto == last_log_id`,
    ///    which makes the send range empty: `start == end`): the probe cannot carry any entry, and
    ///    [`Inflight::probe`] cannot represent an AppendEntries without payload — an empty range
    ///    collapses to [`Inflight::None`]. Pipeline mode is not affected: [`Inflight::LogsSince`]
    ///    is an open-ended stream, and an empty tail is valid.
    ///
    /// [algo]: crate::docs::protocol::replication::log_replication#algorithm-to-find-the-last-matching-log-id-on-a-follower
    /// [`ReplicationHandler::try_purge_log`]: crate::engine::handler::replication_handler::ReplicationHandler::try_purge_log
    /// [`Config::max_in_snapshot_log_to_keep`]: crate::Config::max_in_snapshot_log_to_keep
    /// [`Updater::update_conflicting`]: crate::progress::entry::update::Updater::update_conflicting
    pub(crate) fn next_send(
        &mut self,
        log_state: &mut RaftState<C>,
        max_entries: u64,
    ) -> Result<&Inflight<C>, &Inflight<C>> {
        if !self.data.inflight.is_none() {
            return Err(&self.data.inflight);
        }

        let last_next = log_state.last_log_id().next_index();
        debug_assert!(
            self.data.searching_end <= last_next,
            "expect: searching_end: {} <= last_log_id.next_index: {}",
            self.data.searching_end,
            last_next
        );

        let purge_upto_next = log_state.purge_upto().next_index();
        let inflight_id = log_state.new_inflight_id();

        // A pending probe applies to this send only, including when a snapshot is sent instead.
        let probe_start = self.data.probe_start.take();

        // Snapshot condition 1: all candidate matching positions are purged.
        if self.data.searching_end < purge_upto_next {
            self.data.inflight = Inflight::snapshot(inflight_id);
            return Ok(&self.data.inflight);
        }

        let matching_next = self.matching().next_index();

        if let Some(probe_start) = probe_start {
            // Like a binary-search probe, start no lower than the purge boundary.
            let start = std::cmp::max(probe_start, purge_upto_next);
            // `start + max_entries` can exceed `u64::MAX` near the index limit. Saturating it
            // keeps the cap exact, because `last_next` is at most `u64::MAX`.
            let end = std::cmp::min(start.saturating_add(max_entries), last_next);

            // Responses received since the probe was chosen may have narrowed the search range.
            let in_search_range = matching_next <= start && start <= self.data.searching_end;

            if in_search_range && start < end {
                let prev = log_state.prev_log_id(start);
                let last = log_state.prev_log_id(end);
                self.data.inflight = Inflight::probe(prev, last, inflight_id);
                return Ok(&self.data.inflight);
            }
        }

        let is_probing = matching_next < self.data.searching_end;

        if is_probing {
            // Probe at the binary-search midpoint, but not below the purge boundary.
            // `start <= searching_end` still holds: `mid <= searching_end` by construction,
            // and `purge_upto_next <= searching_end` by snapshot condition 1 above. It is
            // asserted here, not in `validate()`: while the probe is pending, a conflict may
            // lower `searching_end` below `start`.
            let mid = Self::calc_mid(matching_next, self.data.searching_end);
            let start = std::cmp::max(mid, purge_upto_next);
            debug_assert!(
                start <= self.data.searching_end,
                "expect: probe start: {} <= searching_end: {}",
                start,
                self.data.searching_end
            );
            // Saturating, as for the pending probe above.
            let end = std::cmp::min(start.saturating_add(max_entries), last_next);

            // Snapshot condition 2: the leader log is fully purged; there is no entry
            // for the probe to carry.
            if start == end {
                self.data.inflight = Inflight::snapshot(inflight_id);
                return Ok(&self.data.inflight);
            }

            let prev = log_state.prev_log_id(start);
            let last = log_state.prev_log_id(end);
            self.data.inflight = Inflight::probe(prev, last, inflight_id);
        } else {
            // Pipeline: stream every log after the known matching point.
            // Snapshot condition 1 ensured `matching >= purge_upto`: no needed log is purged.
            self.data.inflight = Inflight::LogsSince {
                prev: self.matching.clone(),
                inflight_id,
            };
        }

        Ok(&self.data.inflight)
    }

    fn calc_mid(matching_next: u64, end: u64) -> u64 {
        debug_assert!(matching_next <= end);
        let d = end - matching_next;
        let offset = d / 16 * 8;
        matching_next + offset
    }
}

impl<C> Display for ProgressEntry<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{{P({})[{}, {}), inflight:{}}}",
            self.data.stream_id,
            self.matching().display(),
            self.data.searching_end,
            self.data.inflight,
        )
    }
}

impl<C> Validate for ProgressEntry<C>
where C: RaftTypeConfig
{
    fn validate(&self) -> Result<(), Box<dyn Error>> {
        validit::less_equal!(self.matching().next_index(), self.data.searching_end);

        self.data.inflight.validate()?;

        match &self.data.inflight {
            Inflight::None => {}
            Inflight::Probe { log_id_range, .. } => {
                // matching <= prev_log_id <= last_log_id
                //
                // `next_send()` plans a probe with `prev_log_id.next_index() <= searching_end`,
                // but this is not checked here: a conflict that does not answer the pending
                // probe, such as a heartbeat's, may lower `searching_end` below
                // `prev_log_id.next_index()` and leave the probe pending.
                //
                // In practice this needs a follower that lost logs it had confirmed, such as after
                // a disk wipe, and log reversion allowed for it through
                // `Config::allow_log_reversion` or `Raft::trigger().allow_next_revert()`. Only
                // such a follower answers a heartbeat with a conflict. Without log reversion
                // allowed, `update_conflicting()` fails a `debug_assert!` on a conflict at or
                // below `matching`. The conflict that resets `matching` drops the probe in flight,
                // but the conflict of a request sent before the reset may arrive after the leader
                // has sent a new probe, and it no longer reverts `matching`. For example:
                //
                // 1. With `matching = 5`, a heartbeat is sent and delayed. `matching` then advances to 8, the logs
                //    up to 5 are purged, and the follower loses its log.
                // 2. A commit update sent with `prev_log_id = 8` gets a conflict at 8. It resets `matching` to
                //    `None`, lowers `searching_end` to 8, and drops the plan in flight. The leader later sends the
                //    probe `(5, 13]`, above the purged logs.
                // 3. The heartbeat gets a conflict at 5, which is not below `matching`. It lowers `searching_end`
                //    to 5 and keeps the probe, whose `prev_log_id` is 5. Its own response, a conflict at 5, ends it
                //    later.
                //
                // A conflict never raises `matching`, so the check below still holds after it.
                validit::less_equal!(self.matching(), log_id_range.prev.as_ref());
            }
            Inflight::Snapshot { inflight_id: _ } => {}
            Inflight::LogsSince { .. } => {}
            Inflight::Clear { .. } => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
