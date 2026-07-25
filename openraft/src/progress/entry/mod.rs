pub(crate) mod update;

use std::error::Error;
use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Formatter;
use std::ops::Deref;
use std::ops::DerefMut;

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
    #[allow(dead_code)]
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

    pub(crate) fn matching(&self) -> Option<&LogIdOf<C>> {
        self.matching.as_ref()
    }

    // This method is only used by tests.
    #[allow(dead_code)]
    pub(crate) fn with_inflight(mut self, inflight: Inflight<C>) -> Self {
        debug_assert_eq!(self.inflight, Inflight::None);

        self.inflight = inflight;
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

impl<C> Deref for ProgressEntry<C>
where C: RaftTypeConfig
{
    type Target = ProgressData<C>;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl<C> DerefMut for ProgressEntry<C>
where C: RaftTypeConfig
{
    fn deref_mut(&mut self) -> &mut Self::Target {
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
        match &self.inflight {
            Inflight::None => false,
            Inflight::Logs { log_id_range, .. } => {
                let lid = Some(upto);
                lid > log_id_range.prev.as_ref()
            }
            Inflight::Snapshot { inflight_id: _ } => false,
            Inflight::LogsSince { prev, .. } => {
                // All logs after prev are inflight in streaming mode
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
    ///   determined. Send a fixed range of logs `(prev, last]` ([`Inflight::Logs`]) with `prev` at
    ///   a binary-search midpoint: a success response raises `matching`, a conflict response lowers
    ///   `searching_end`, until the range collapses.
    ///
    /// - **Pipeline** (`matching.next_index() == searching_end`): the matching point is exactly
    ///   `matching`. Stream all logs after it, with no fixed upper bound ([`Inflight::LogsSince`]).
    ///
    /// Purging constrains what AppendEntries can be built: log entries at index
    /// `<= purge_upto` are deleted, and only the log id at `purge_upto` itself is still
    /// known, as the snapshot's last log id. Thus the lowest usable `prev` is `purge_upto`.
    /// A snapshot must be sent instead of logs in exactly two situations:
    ///
    /// 1. `searching_end < purge_upto_next`, in either regime: every candidate matching position
    ///    lies strictly below the purge boundary. The lowest possible probe, `prev = purge_upto`,
    ///    sits at an index `>= searching_end` — a position already known not to match — so the
    ///    follower would reply with a conflict at that same index, which carries no new information
    ///    and is discarded (see [`Updater::update_conflicting`]): log replication cannot make
    ///    progress. `searching_end == purge_upto_next` is excluded: `prev = purge_upto` is then at
    ///    index `searching_end - 1`, still a candidate position worth probing.
    ///
    /// 2. Probing while the leader log is fully purged (`purge_upto == last_log_id`, which makes
    ///    the send range empty: `start == end`): the probe cannot carry any entry, and
    ///    [`Inflight::logs`] cannot represent an AppendEntries without payload — an empty range
    ///    collapses to [`Inflight::None`]. Pipeline mode is not affected: [`Inflight::LogsSince`]
    ///    is an open-ended stream, and an empty tail is valid.
    ///
    /// [algo]: crate::docs::protocol::replication::log_replication#algorithm-to-find-the-last-matching-log-id-on-a-follower
    /// [`Updater::update_conflicting`]: crate::progress::entry::update::Updater::update_conflicting
    pub(crate) fn next_send(
        &mut self,
        log_state: &mut RaftState<C>,
        max_entries: u64,
    ) -> Result<&Inflight<C>, &Inflight<C>> {
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

        let purge_upto_next = log_state.purge_upto().next_index();
        let inflight_id = log_state.new_inflight_id();

        // Snapshot condition 1: all candidate matching positions are purged.
        if self.searching_end < purge_upto_next {
            self.inflight = Inflight::snapshot(inflight_id);
            return Ok(&self.inflight);
        }

        let matching_next = self.matching().next_index();
        let is_probing = matching_next < self.searching_end;

        if is_probing {
            // Probe at the binary-search midpoint, but not below the purge boundary.
            // `start <= searching_end` still holds: `mid <= searching_end` by construction,
            // and `purge_upto_next <= searching_end` by snapshot condition 1 above.
            let mid = Self::calc_mid(matching_next, self.searching_end);
            let start = std::cmp::max(mid, purge_upto_next);
            let end = std::cmp::min(start + max_entries, last_next);

            // Snapshot condition 2: the leader log is fully purged; there is no entry
            // for the probe to carry.
            if start == end {
                self.inflight = Inflight::snapshot(inflight_id);
                return Ok(&self.inflight);
            }

            let prev = log_state.prev_log_id(start);
            let last = log_state.prev_log_id(end);
            self.inflight = Inflight::logs(prev, last, inflight_id);
        } else {
            // Pipeline: stream every log after the known matching point.
            // Snapshot condition 1 ensured `matching >= purge_upto`: no needed log is purged.
            self.inflight = Inflight::LogsSince {
                prev: self.matching.clone(),
                inflight_id,
            };
        }

        Ok(&self.inflight)
    }

    /// Return the index range (`[start,end]`) of the first log in the next AppendEntries.
    ///
    /// The returned range is left close and right close.
    #[allow(dead_code)]
    pub(crate) fn sending_start(&self) -> (u64, u64) {
        let mid = Self::calc_mid(self.matching().next_index(), self.searching_end);
        (mid, self.searching_end)
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
            self.stream_id,
            self.matching().display(),
            self.searching_end,
            self.inflight,
        )
    }
}

impl<C> Validate for ProgressEntry<C>
where C: RaftTypeConfig
{
    fn validate(&self) -> Result<(), Box<dyn Error>> {
        validit::less_equal!(self.matching().next_index(), self.searching_end);

        self.inflight.validate()?;

        match &self.inflight {
            Inflight::None => {}
            Inflight::Logs { log_id_range, .. } => {
                // matching <= prev_log_id              <= last_log_id
                //             prev_log_id.next_index() <= searching_end
                validit::less_equal!(self.matching(), log_id_range.prev.as_ref());
                validit::less_equal!(log_id_range.prev.next_index(), self.searching_end);
            }
            Inflight::Snapshot { inflight_id: _ } => {}
            Inflight::LogsSince { .. } => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
