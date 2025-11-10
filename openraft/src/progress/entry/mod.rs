pub(crate) mod update;

use std::borrow::Borrow;
use std::error::Error;
use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Formatter;

use validit::Validate;

use crate::LogIdOptionExt;
use crate::RaftTypeConfig;
use crate::display_ext::DisplayOptionExt;
use crate::engine::EngineConfig;
use crate::progress::entry::update::Updater;
use crate::progress::inflight::Inflight;
use crate::raft_state::LogStateReader;
use crate::type_config::alias::LogIdOf;

/// State of replication to a target node.
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
pub(crate) struct ProgressEntry<C>
where C: RaftTypeConfig
{
    /// The id of the last matching log on the target following node.
    pub(crate) matching: Option<LogIdOf<C>>,

    /// The data being transmitted in flight.
    ///
    /// A non-none inflight expects a response when the data was successfully sent or failed.
    pub(crate) inflight: Inflight<C>,

    /// One plus the max log index on the following node that might match the leader log.
    pub(crate) searching_end: u64,

    /// If true, reset the progress by setting [`Self::matching`] to `None` when the follower's
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
    pub(crate) fn new(matching: Option<LogIdOf<C>>) -> Self {
        Self {
            matching: matching.clone(),
            inflight: Inflight::None,
            searching_end: matching.next_index(),
            allow_log_reversion: false,
        }
    }

    /// Create a progress entry that does not have any matching log id.
    ///
    /// It's going to initiate a binary search to find the minimal matching log id.
    pub(crate) fn empty(end: u64) -> Self {
        Self {
            matching: None,
            inflight: Inflight::None,
            searching_end: end,
            allow_log_reversion: false,
        }
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

    pub(crate) fn matching(&self) -> Option<&LogIdOf<C>> {
        self.matching.as_ref()
    }

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
            Inflight::Snapshot => false,
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
        log_state: &impl LogStateReader<C>,
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
            self.inflight = Inflight::snapshot();
            return Ok(&self.inflight);
        }

        // Replicate by logs.
        // Run a binary search to find the matching log id, if matching log id is not determined.
        let mut start = Self::calc_mid(self.matching().next_index(), self.searching_end);
        if start < purge_upto_next {
            start = purge_upto_next;
        }

        let end = std::cmp::min(start + max_entries, last_next);

        if start == end {
            self.inflight = Inflight::None;
            return Err(&self.inflight);
        }

        let prev = log_state.prev_log_id(start);
        let last = log_state.prev_log_id(end);

        self.inflight = Inflight::logs(prev, last);

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

impl<C> Borrow<Option<LogIdOf<C>>> for ProgressEntry<C>
where C: RaftTypeConfig
{
    fn borrow(&self) -> &Option<LogIdOf<C>> {
        &self.matching
    }
}

impl<C> Display for ProgressEntry<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{{[{}, {}), inflight:{}}}",
            self.matching().display(),
            self.searching_end,
            self.inflight
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
            Inflight::Snapshot => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
