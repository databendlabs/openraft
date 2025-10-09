use std::time::Duration;

/// An additional argument to the [`RaftNetwork`] methods to allow applications to customize
/// networking behaviors.
///
/// [`RaftNetwork`]: `crate::network::RaftNetwork`
#[derive(Clone, Debug)]
pub struct RPCOption {
    /// The expected time-to-last for an RPC.
    ///
    /// The caller will cancel an RPC if it takes longer than this duration.
    hard_ttl: Duration,

    /// The size of the snapshot chunk.
    pub(crate) snapshot_chunk_size: Option<usize>,
}

impl RPCOption {
    /// Create a new RPCOption with the given hard TTL.
    pub fn new(hard_ttl: Duration) -> Self {
        Self {
            hard_ttl,
            snapshot_chunk_size: None,
        }
    }

    /// The moderate max interval an RPC should last for.
    ///
    /// The [`hard_ttl()`] and `soft_ttl()` methods of `RPCOption` set the hard limit and the
    /// moderate limit of the duration for which an RPC should run. Once the `soft_ttl()` ends,
    /// the RPC implementation should start to gracefully cancel the RPC, and once the
    /// `hard_ttl()` ends, Openraft will terminate the ongoing RPC at once.
    ///
    /// `soft_ttl` is smaller than [`hard_ttl()`] so that the RPC implementation can cancel the RPC
    /// gracefully after `soft_ttl` and before `hard_ttl`.
    ///
    /// `soft_ttl` is 3/4 of `hard_ttl` but it may change in future, do not rely on this ratio.
    ///
    /// [`hard_ttl()`]: `Self::hard_ttl`
    pub fn soft_ttl(&self) -> Duration {
        self.hard_ttl * 3 / 4
    }

    /// The hard limit of the interval an RPC should last for.
    ///
    /// When exceeding this limit, the RPC will be dropped by Openraft at once.
    pub fn hard_ttl(&self) -> Duration {
        self.hard_ttl
    }

    /// Get the recommended size of the snapshot chunk for transport.
    pub fn snapshot_chunk_size(&self) -> Option<usize> {
        self.snapshot_chunk_size
    }
}
