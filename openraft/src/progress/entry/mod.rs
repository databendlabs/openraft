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

    /// Initialize a replication action: sending log entries or sending snapshot.
    ///
    /// If there is an action in progress, i.e., `inflight` is not None, it returns an `Err`
    /// containing the current `inflight` data.
    ///
    /// See: [Algorithm to find the last matching log id on a Follower][algo].
    ///
    /// [algo]: crate::docs::protocol::replication::log_replication#algorithm-to-find-the-last-matching-log-id-on-a-follower
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

        let purge_upto_next = {
            let purge_upto = log_state.purge_upto();
            purge_upto.next_index()
        };

        // `searching_end` is the max value for `start`.

        // The log the follower needs is purged.
        // Replicate by snapshot.
        if self.searching_end < purge_upto_next {
            let inflight_id = log_state.new_inflight_id();
            self.inflight = Inflight::snapshot(inflight_id);
            return Ok(&self.inflight);
        }

        let matching_next = self.matching().next_index();

        // Replicate by logs.
        // Run a binary search to find the matching log id, if matching log id is not determined.
        let mut start = Self::calc_mid(matching_next, self.searching_end);
        if start < purge_upto_next {
            start = purge_upto_next;
        }

        // Enter pipeline mode
        if start == matching_next && matching_next == self.searching_end {
            let prev = log_state.prev_log_id(start);
            let inflight_id = log_state.new_inflight_id();
            self.inflight = Inflight::LogsSince { prev, inflight_id };
            return Ok(&self.inflight);
        }

        let end = std::cmp::min(start + max_entries, last_next);

        if start == end {
            self.inflight = Inflight::None;
            return Err(&self.inflight);
        }

        let prev = log_state.prev_log_id(start);
        let last = log_state.prev_log_id(end);

        let inflight_id = log_state.new_inflight_id();
        self.inflight = Inflight::logs(prev, last, inflight_id);

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
