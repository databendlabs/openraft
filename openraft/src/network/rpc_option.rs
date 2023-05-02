use std::time::Duration;

/// An additional argument to the [`RaftNetwork`] methods to allow applications to customize
/// networking behaviors.
///
/// [`RaftNetwork`]: `crate::network::RaftNetwork`
pub struct RPCOption {
    /// The expected time-to-last for an RPC.
    ///
    /// The caller will cancel an RPC if it takes longer than this duration.
    ttl: Duration,
}

impl RPCOption {
    pub fn new(ttl: Duration) -> Self {
        Self { ttl }
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }
}
