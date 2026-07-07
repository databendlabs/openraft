//! RuntimeConfigHandle is an interface to change Raft runtime config.

use std::sync::atomic::Ordering;

use crate::RaftTypeConfig;
use crate::raft::RaftInner;

/// RuntimeConfigHandle is an interface to update runtime config.
///
/// These configs are mainly designed for testing purposes and special use cases.
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

    /// Enable or disable heartbeat messages when a leader has no more log to replicate.
    ///
    /// Note that the follower's leader-lease will not be renewed if it does receive messages from
    /// the leader, and it will start election (if `Self::elect()` is enabled) when the lease timed
    /// out.
    pub fn heartbeat(&self, enabled: bool) {
        self.raft_inner.runtime_config.enable_heartbeat.store(enabled, Ordering::Relaxed);
    }

    /// Enable or disable election for a follower when its leader lease timed out.
    pub fn elect(&self, enabled: bool) {
        self.raft_inner.runtime_config.enable_elect.store(enabled, Ordering::Relaxed);
    }

    /// Enable or disable the Pre-Vote round that precedes a real election.
    ///
    /// When enabled, a follower asks peers whether they would grant it a vote before incrementing
    /// its term, avoiding term inflation by a node that cannot currently win. See
    /// [`Config::enable_pre_vote`](crate::Config::enable_pre_vote).
    pub fn pre_vote(&self, enabled: bool) {
        self.raft_inner.runtime_config.enable_pre_vote.store(enabled, Ordering::Relaxed);
    }
}
