use std::time::Duration;

/// An additional argument to the [`RaftNetworkV2`] methods to allow applications to customize
/// networking behaviors.
///
/// [`RaftNetworkV2`]: `crate::network::RaftNetworkV2`
#[derive(Clone, Debug)]
pub struct RPCOption {
    /// The hard upper bound for a request-response RPC.
    ///
    /// Openraft may shut down an in-flight RPC once this duration has elapsed. Network
    /// implementations may read this value, but should use [`RPCOption::soft_ttl`] to configure
    /// their transport timeout, deadline, or equivalent mechanism.
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

    /// The timeout budget a network implementation should enforce.
    ///
    /// The [`hard_ttl()`] and `soft_ttl()` methods of `RPCOption` set the hard limit and the
    /// application-controlled timeout of a request-response RPC. The network implementation should
    /// time out the RPC once `soft_ttl()` ends.
    ///
    /// `soft_ttl` is smaller than [`hard_ttl()`] so that the RPC implementation can return a
    /// timeout error before Openraft may shut down the in-flight RPC at the hard limit.
    ///
    /// For a long-lived stream, [`hard_ttl()`] is not a limit on the stream lifetime. The transport
    /// should apply `soft_ttl()` to its setup, idle timeout, keepalive, read deadline, or reconnect
    /// policy.
    ///
    /// `soft_ttl` is 3/4 of `hard_ttl` but it may change in future, do not rely on this ratio.
    ///
    /// [`hard_ttl()`]: `Self::hard_ttl`
    pub fn soft_ttl(&self) -> Duration {
        self.hard_ttl * 3 / 4
    }

    /// The hard upper bound Openraft may use to shut down an in-flight RPC.
    ///
    /// Network implementations should use [`RPCOption::soft_ttl`] to control their own timeout.
    pub fn hard_ttl(&self) -> Duration {
        self.hard_ttl
    }

    /// Get the recommended size of the snapshot chunk for transport.
    pub fn snapshot_chunk_size(&self) -> Option<usize> {
        self.snapshot_chunk_size
    }
}
