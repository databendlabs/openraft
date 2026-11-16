//! RuntimeConfigHandle is an interface to change Raft runtime config.

use std::sync::atomic::Ordering;

use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::async_runtime::watch::WatchSender;
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
    /// Disabling tick stops a leader's heartbeats and the periodic election checks of a
    /// candidate, a learner, and a leader whose leadership transfer timed out. It does not stop a
    /// follower's election, which [`Self::elect`] controls. While disabled, the ticker parks and
    /// causes no wakeups; re-enabling it resumes ticking, with the first tick at most one tick
    /// period later. [`Trigger::tick`] still runs a timer check while the ticker is disabled.
    ///
    /// [`Trigger::tick`]: crate::raft::trigger::Trigger::tick
    #[since(version = "0.10.1", change = "no longer stops a follower's election")]
    pub fn tick(&self, enabled: bool) {
        let enable_tick = self.raft_inner.runtime_config.enable_tick.lock().unwrap();
        enable_tick.send_if_different(enabled);
    }

    /// Enable or disable heartbeat messages when a leader has no more log to replicate.
    ///
    /// Note that the follower's leader-lease will not be renewed if it does receive messages from
    /// the leader, and it will start election (if `Self::elect()` is enabled) when the lease timed
    /// out.
    pub fn heartbeat(&self, enabled: bool) {
        self.raft_inner.runtime_config.enable_heartbeat.store(enabled, Ordering::Relaxed);
    }

    /// Enable or disable automatic elections, such as a follower's when its leader lease timed out.
    ///
    /// A follower's election runs on its own election timer, whatever [`Self::tick`] says.
    /// Re-enabling elections takes effect the next time `RaftCore` wakes up: on the next internal
    /// tick, at most one tick period later, or on the next message. A follower with [`Self::tick`]
    /// disabled and no incoming messages does not elect until something wakes it, such as
    /// [`Trigger::tick`].
    ///
    /// [`Trigger::tick`]: crate::raft::trigger::Trigger::tick
    #[since(version = "0.10.1", change = "controls a follower's election without `tick`")]
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
