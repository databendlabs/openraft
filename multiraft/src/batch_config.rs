use std::num::NonZeroUsize;
use std::time::Duration;

use openraft::openraft_macros::since;

/// Limits of the batches that [`BatchingRouter`](crate::BatchingRouter) sends to one peer node.
///
/// Only empty AppendEntries requests are batched, so every item is small and a count bounds the
/// size of a batch.
#[since(version = "0.10.1")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchConfig {
    /// The most requests in one batch.
    ///
    /// Default: 1024.
    pub max_items: NonZeroUsize,

    /// How long a batch waits after its first request for more requests.
    ///
    /// A batch is sent earlier when it is full, or when the deadline of one of its requests
    /// comes first; a later request never delays it. Default: 1 ms.
    pub max_delay: Duration,

    /// The most batches in flight to one peer at a time.
    ///
    /// Requests queue up while this many batches wait for replies. Default: 4.
    pub max_in_flight: NonZeroUsize,

    /// The most requests queued for one peer; a request waits for room beyond it.
    ///
    /// Default: 4096.
    pub max_queued: NonZeroUsize,
}

impl Default for BatchConfig {
    fn default() -> Self {
        Self {
            max_items: NonZeroUsize::new(1024).unwrap(),
            max_delay: Duration::from_millis(1),
            max_in_flight: NonZeroUsize::new(4).unwrap(),
            max_queued: NonZeroUsize::new(4096).unwrap(),
        }
    }
}
