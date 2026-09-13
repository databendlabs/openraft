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

    /// A one-shot probe at this leader's first entry.
    pub(crate) initial_probe_index: Option<u64>,

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

    /// Try `index` before falling back to the normal binary search.
    pub(crate) fn with_initial_probe(mut self, index: u64) -> Self {
        self.data.initial_probe_index = Some(index);
        self
    }

    pub(crate) fn matching(&self) -> Option<&LogIdOf<C>> {
        self.matching.as_ref()
    }

    // This method is only used by tests.
    #[allow(dead_code)]
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
            initial_probe_index: None,
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

        // Snapshot condition 1: all candidate matching positions are purged.
        if self.data.searching_end < purge_upto_next {
            self.data.inflight = Inflight::snapshot(inflight_id);
            return Ok(&self.data.inflight);
        }

        // Probe this leader's first entry before binary search.
        if let Some(probe_index) = self.data.initial_probe_index.take() {
            let start = std::cmp::max(probe_index, purge_upto_next);
            let end = std::cmp::min(start + max_entries, last_next);
            if start < end {
                let prev = log_state.prev_log_id(start);
                let last = log_state.prev_log_id(end);
                self.data.inflight = Inflight::logs(prev, last, inflight_id);
                return Ok(&self.data.inflight);
            }
        }

        let matching_next = self.matching().next_index();
        let is_probing = matching_next < self.data.searching_end;

        if is_probing {
            // Probe at the binary-search midpoint, but not below the purge boundary.
            // `start <= searching_end` still holds: `mid <= searching_end` by construction,
            // and `purge_upto_next <= searching_end` by snapshot condition 1 above.
            let mid = Self::calc_mid(matching_next, self.data.searching_end);
            let start = std::cmp::max(mid, purge_upto_next);
            let end = std::cmp::min(start + max_entries, last_next);

            // Snapshot condition 2: the leader log is fully purged; there is no entry
            // for the probe to carry.
            if start == end {
                self.data.inflight = Inflight::snapshot(inflight_id);
                return Ok(&self.data.inflight);
            }

            let prev = log_state.prev_log_id(start);
            let last = log_state.prev_log_id(end);
            self.data.inflight = Inflight::logs(prev, last, inflight_id);
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

    /// Return the index range (`[start,end]`) of the first log in the next AppendEntries.
    ///
    /// The returned range is left close and right close.
    #[allow(dead_code)]
    pub(crate) fn sending_start(&self) -> (u64, u64) {
        let mid = Self::calc_mid(self.matching().next_index(), self.data.searching_end);
        (mid, self.data.searching_end)
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
            Inflight::Logs { log_id_range, .. } => {
                // matching <= prev_log_id              <= last_log_id
                //             prev_log_id.next_index() <= searching_end
                validit::less_equal!(self.matching(), log_id_range.prev.as_ref());
                validit::less_equal!(log_id_range.prev.next_index(), self.data.searching_end);
            }
            Inflight::Snapshot { inflight_id: _ } => {}
            Inflight::LogsSince { .. } => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
