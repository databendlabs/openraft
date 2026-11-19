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

    /// The entries of the range, read before the batch is applied.
    ///
    /// Read errors are kept as items, so that they reach `apply()` at the same position as when
    /// the entries are streamed.
    pub(crate) entries: Vec<Result<EntryOf<C>, io::Error>>,
}

impl<C> ApplyBatch<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(first: LogIdOf<C>, last: LogIdOf<C>, client_resp_channels: Vec<(u64, CoreResponder<C>)>) -> Self {
        Self {
            first,
            last,
            client_resp_channels,
            entries: Vec::new(),
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

    /// Append `next`, a batch that starts at [`Self::end`], with its entries and responders.
    pub(crate) fn extend(&mut self, next: ApplyBatch<C>) {
        debug_assert_eq!(self.end(), next.since(), "an extension continues the batch");
        self.last = next.last;
        self.client_resp_channels.extend(next.client_resp_channels);
        self.entries.extend(next.entries);
    }

    pub(crate) fn entry_count(&self) -> u64 {
        let since = self.since();
        let end = self.end();
        end - since
    }
}
