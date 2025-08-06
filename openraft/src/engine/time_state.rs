use std::time::Duration;

#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
pub(crate) struct Config {
    /// The time interval after which the next election will be initiated once the current lease has
    /// expired.
    pub(crate) election_timeout: Duration,

    /// If this node has a smaller last-log-id than others, it will be less likely to be elected as
    /// a leader. In this case, it is necessary to sleep for a longer period of time
    /// `smaller_log_timeout` so that other nodes with a greater last-log-id have a chance to elect
    /// themselves.
    ///
    /// Note that this value should be greater than the `election_timeout` of every other node.
    pub(crate) smaller_log_timeout: Duration,

    /// The duration of an active leader's lease for a follower.
    ///
    /// When a follower or learner perceives an active leader, such as by receiving an AppendEntries
    /// message(including heartbeat), it should not grant the vote to another candidate during this
    /// period. This prevents unnecessary elections when the leader is still active and helps
    /// maintain cluster stability.
    ///
    /// The leader uses this value to determine how long it can safely assume leadership without
    /// sending heartbeats. A leader tracks the clock time acknowledged by followers to establish
    /// a quorum-acknowledged time, ensuring no new leader can be elected before this lease expires.
    pub(crate) leader_lease: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            election_timeout: Duration::from_millis(150),
            smaller_log_timeout: Duration::from_millis(200),
            leader_lease: Duration::from_millis(150),
        }
    }
}
