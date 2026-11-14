use std::io;

use crate::RaftTypeConfig;
use crate::raft::responder::core_responder::CoreResponder;
use crate::type_config::alias::EntryOf;
use crate::type_config::alias::LogIdOf;

/// A range of committed entries that the state-machine worker applies with one `apply()` call.
pub(crate) struct ApplyBatch<C>
where C: RaftTypeConfig
{
    /// The first log id to apply, inclusive.
    pub(crate) first: LogIdOf<C>,

    /// The last log id to apply, inclusive.
    pub(crate) last: LogIdOf<C>,

    /// Client responders as `(log_index, responder)` pairs, sorted by log index.
    pub(crate) client_resp_channels: Vec<(u64, CoreResponder<C>)>,

    /// Entries read while the previous batch was applying, or `None` to read them when this batch
    /// is applied.
    ///
    /// Read errors are kept as items, so that they reach `apply()` at the same position as when
    /// the entries are streamed.
    pub(crate) prefetched: Option<Vec<Result<EntryOf<C>, io::Error>>>,
}

impl<C> ApplyBatch<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(first: LogIdOf<C>, last: LogIdOf<C>, client_resp_channels: Vec<(u64, CoreResponder<C>)>) -> Self {
        Self {
            first,
            last,
            client_resp_channels,
            prefetched: None,
        }
    }

    /// Append `other`, the batch that starts right after this one, and its prefetched entries.
    pub(crate) fn append(&mut self, other: ApplyBatch<C>) {
        debug_assert_eq!(self.end(), other.since(), "appended batch must continue this one");
        self.last = other.last;
        self.client_resp_channels.extend(other.client_resp_channels);
        match (&mut self.prefetched, other.prefetched) {
            (Some(entries), Some(more)) => entries.extend(more),
            // Without entries for the whole range, the batch reads them all when applied.
            (prefetched, _) => *prefetched = None,
        }
    }

    /// The index of the first entry, inclusive.
    pub(crate) fn since(&self) -> u64 {
        self.first.index()
    }

    /// The index after the last entry, exclusive.
    pub(crate) fn end(&self) -> u64 {
        self.last.index() + 1
    }
}
