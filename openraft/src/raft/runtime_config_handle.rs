//! RuntimeConfigHandle is an interface to change Raft runtime config.

use std::sync::atomic::Ordering;

use crate::raft::RaftInner;
use crate::RaftTypeConfig;

/// RuntimeConfigHandle is an interface to update runtime config.
///
/// These config are mainly designed for testing purpose and special use cases.
/// Usually you don't need to change runtime config.
pub struct RuntimeConfigHandle<'r, C>
where C: RaftTypeConfig
{
    raft_inner: &'r RaftInner<C>,
}

impl<'r, C> RuntimeConfigHandle<'r, C>
where C: RaftTypeConfig
{
    pub(in crate::raft) fn new(raft_inner: &'r RaftInner<C>) -> Self {
        Self { raft_inner }
    }

    /// Enable or disable raft internal ticker.
    ///
    /// Disabling tick will disable election and heartbeat.
    pub fn tick(&self, enabled: bool) {
        self.raft_inner.tick_handle.enable(enabled);
    }

    /// Enable or disable heartbeat message when a leader has no more log to replicate.
    ///
    /// Note that the follower's leader-lease will not be renewed if it does receive message from
    /// the leader, and it will start election(if `Self::elect()` is enabled) when the lease timed
    /// out.
    pub fn heartbeat(&self, enabled: bool) {
        self.raft_inner.runtime_config.enable_heartbeat.store(enabled, Ordering::Relaxed);
    }

    /// Enable or disable election for a follower when its leader lease timed out.
    pub fn elect(&self, enabled: bool) {
        self.raft_inner.runtime_config.enable_elect.store(enabled, Ordering::Relaxed);
    }
}
