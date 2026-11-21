//! RuntimeConfigHandle is an interface to change Raft runtime config.

use std::sync::atomic::Ordering;

use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::raft::RaftInner;

/// RuntimeConfigHandle is an interface to update and read runtime config.
///
/// These configs are mainly designed for testing purposes and special use cases.
/// Usually you don't need to change runtime config.
///
/// Changing a switch wakes `RaftCore`, so that the change takes effect at once, even when no
/// message arrives.
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

    /// Enable or disable a leader's heartbeat timer.
    ///
    /// Disabling it stops a leader's automatic heartbeats. It does not stop elections, which
    /// [`Self::elect`] controls. While disabled, the timer causes no wakeups; re-enabling it wakes
    /// `RaftCore`, which sends a heartbeat that is already due at once. [`Trigger::tick`] still
    /// runs a timer check while it is disabled.
    ///
    /// [`Trigger::tick`]: crate::raft::trigger::Trigger::tick
    #[since(version = "0.10.1", change = "pauses the heartbeat timer that replaces the ticker")]
    #[since(version = "0.10.1", change = "no longer stops elections")]
    pub fn tick(&self, enabled: bool) {
        self.raft_inner.runtime_config.enable_tick.store(enabled, Ordering::Relaxed);
        self.raft_inner.wake_core();
    }

    /// Enable or disable heartbeat messages when a leader has no more log to replicate.
    ///
    /// Note that the follower's leader-lease will not be renewed if it does receive messages from
    /// the leader, and it will start election (if `Self::elect()` is enabled) when the lease timed
    /// out.
    pub fn heartbeat(&self, enabled: bool) {
        self.raft_inner.runtime_config.enable_heartbeat.store(enabled, Ordering::Relaxed);
        self.raft_inner.wake_core();
    }

    /// Enable or disable automatic elections, such as a follower's when its leader lease timed out.
    ///
    /// Elections run on their own election timer, whatever [`Self::tick`] says. Re-enabling
    /// elections wakes `RaftCore`, so a node whose election is already due starts it at once, even
    /// with [`Self::tick`] disabled and no incoming messages.
    ///
    /// See [a voter that does not campaign](crate::docs::cluster_control::leadership).
    #[since(version = "0.10.1", change = "wakes `RaftCore` to apply the change")]
    #[since(version = "0.10.1", change = "controls elections without `tick`")]
    pub fn elect(&self, enabled: bool) {
        self.raft_inner.runtime_config.enable_elect.store(enabled, Ordering::Relaxed);
        self.raft_inner.wake_core();
    }

    /// Enable or disable the Pre-Vote round that precedes a real election.
    ///
    /// When enabled, a follower asks peers whether they would grant it a vote before incrementing
    /// its term, avoiding term inflation by a node that cannot currently win. See
    /// [`Config::enable_pre_vote`](crate::Config::enable_pre_vote).
    pub fn pre_vote(&self, enabled: bool) {
        self.raft_inner.runtime_config.enable_pre_vote.store(enabled, Ordering::Relaxed);
        self.raft_inner.wake_core();
    }

    /// Return whether a leader's heartbeat timer is enabled, as set by [`Self::tick`].
    ///
    /// It starts as [`Config::enable_tick`](crate::Config::enable_tick).
    #[since(version = "0.10.2")]
    pub fn is_tick_enabled(&self) -> bool {
        self.raft_inner.runtime_config.enable_tick.load(Ordering::Relaxed)
    }

    /// Return whether an idle leader sends heartbeats, as set by [`Self::heartbeat`].
    ///
    /// It starts as [`Config::enable_heartbeat`](crate::Config::enable_heartbeat).
    #[since(version = "0.10.2")]
    pub fn is_heartbeat_enabled(&self) -> bool {
        self.raft_inner.runtime_config.enable_heartbeat.load(Ordering::Relaxed)
    }

    /// Return whether this node starts an election when its election timer expires, as set by
    /// [`Self::elect`].
    ///
    /// It starts as [`Config::enable_elect`](crate::Config::enable_elect). With it disabled, a
    /// voter still grants votes and still takes over when a leader transfers leadership to it;
    /// [`Trigger::elect`](crate::raft::trigger::Trigger::elect) starts an election regardless.
    ///
    /// A node that was the leader before a restart still campaigns at startup, unless
    /// [`Config::enable_leader_restore`](field@crate::Config::enable_leader_restore) is
    /// `Some(false)`.
    #[since(version = "0.10.2")]
    pub fn is_elect_enabled(&self) -> bool {
        self.raft_inner.runtime_config.enable_elect.load(Ordering::Relaxed)
    }

    /// Return whether an automatic election starts with a Pre-Vote round, as set by
    /// [`Self::pre_vote`].
    ///
    /// It starts as [`Config::enable_pre_vote`](crate::Config::enable_pre_vote), where `None`
    /// means disabled.
    #[since(version = "0.10.2")]
    pub fn is_pre_vote_enabled(&self) -> bool {
        self.raft_inner.runtime_config.enable_pre_vote.load(Ordering::Relaxed)
    }
}
