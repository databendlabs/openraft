use std::fmt;
use std::time::Duration;

use openraft_macros::since;

use crate::raft::ReadPolicy;

/// Configures the quorum-acknowledgement requirement used to obtain a [`Linearizer`].
///
/// A read may proceed when the leader has an acknowledgement from a quorum for RPCs sent at
/// `quorum_acked_at` and:
///
/// ```text
/// requested_at < quorum_acked_at + effective_max_quorum_ack_age
/// ```
///
/// `effective_max_quorum_ack_age` is `max_quorum_ack_age` capped at the configured leader lease, or
/// the leader lease when `max_quorum_ack_age` is absent. A zero duration requires a quorum to
/// acknowledge an RPC sent after `requested_at`, as in ReadIndex. A nonzero duration allows a
/// recent quorum acknowledgement to be reused as a leader lease.
///
/// `heartbeat_if_quorum_ack_stale` controls whether [`RaftCore`] immediately broadcasts heartbeats
/// when the recorded acknowledgement is missing or reaches the effective age limit. It does not
/// control whether the read waits: a stale read may also be satisfied by a periodic heartbeat or
/// replication acknowledgement.
///
/// `wait_timeout` bounds how long such a read waits for a qualifying acknowledgement. The leader
/// lease bounds `max_quorum_ack_age` only; it is the default wait, not a limit on it.
///
/// [`RaftCore`]: crate::core::RaftCore
#[since(version = "0.10.0", change = "renamed from `LinearizableReadRequest`")]
#[since(version = "0.10.0")]
#[derive(Debug, Clone)]
pub struct LinearizerOption {
    /// The maximum age of a quorum acknowledgement accepted by this read.
    ///
    /// If the latest quorum-acknowledged RPC was sent at `quorum_acked_at`, the read may proceed
    /// when `requested_at < quorum_acked_at + max_quorum_ack_age`.
    ///
    /// `None` uses the configured leader lease. `Some(duration)` overrides that value for this read
    /// and is capped at the configured leader lease. `Some(Duration::ZERO)` requires a quorum
    /// acknowledgement for an RPC sent after `requested_at`.
    pub(crate) max_quorum_ack_age: Option<Duration>,

    /// Whether to broadcast heartbeats immediately when the latest quorum acknowledgement is
    /// missing or reaches the effective maximum age.
    pub(crate) heartbeat_if_quorum_ack_stale: bool,

    /// The maximum time this read waits for a qualifying quorum acknowledgement.
    ///
    /// The wait applies only to a read that cannot be answered from the recorded acknowledgement.
    /// `None` uses the configured leader lease.
    /// `Some(Duration::ZERO)` fails the read at once instead of queueing it.
    ///
    /// The wait starts when [`RaftCore`] begins handling the request rather than when the caller
    /// submits it, and it has no upper bound: a large value keeps the read in [`RaftCore`] memory
    /// until it elapses.
    ///
    /// [`RaftCore`]: crate::core::RaftCore
    pub(crate) wait_timeout: Option<Duration>,
}

impl fmt::Display for LinearizerOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "LinearizerOption {{ max_quorum_ack_age: {:?}, heartbeat_if_quorum_ack_stale: {}, wait_timeout: {:?} }}",
            self.max_quorum_ack_age, self.heartbeat_if_quorum_ack_stale, self.wait_timeout
        )
    }
}

impl LinearizerOption {
    /// Creates a linearizer option.
    ///
    /// `max_quorum_ack_age` is the maximum age of a quorum acknowledgement accepted by this read.
    /// `None` uses the configured leader lease. Values exceeding the leader lease are capped at
    /// the leader lease when the option is handled.
    ///
    /// If `heartbeat_if_quorum_ack_stale` is `true`, the leader immediately broadcasts heartbeats
    /// when its latest quorum acknowledgement is too old for this read. Otherwise, the read waits
    /// for a periodic heartbeat or replication acknowledgement without initiating one.
    #[since(version = "0.10.0")]
    pub fn new(max_quorum_ack_age: Option<Duration>, heartbeat_if_quorum_ack_stale: bool) -> Self {
        Self {
            max_quorum_ack_age,
            heartbeat_if_quorum_ack_stale,
            wait_timeout: None,
        }
    }

    /// Sets how long this read waits for a qualifying quorum acknowledgement.
    ///
    /// The wait applies only to a read that needs a newer quorum acknowledgement, and it replaces
    /// the default wait of one leader lease. `Duration::ZERO` fails such a read at once instead of
    /// queueing it. The value is not capped, so a long wait keeps the read in memory on the leader
    /// until it elapses or the leader steps down.
    #[since(version = "0.10.0")]
    pub fn with_wait_timeout(mut self, wait_timeout: Duration) -> Self {
        self.wait_timeout = Some(wait_timeout);
        self
    }

    pub(crate) fn effective_max_quorum_ack_age(&self, leader_lease: Duration) -> Duration {
        let requested_age = self.max_quorum_ack_age.unwrap_or(leader_lease);
        requested_age.min(leader_lease)
    }

    pub(crate) fn effective_wait_timeout(&self, leader_lease: Duration) -> Duration {
        self.wait_timeout.unwrap_or(leader_lease)
    }

    pub(crate) fn from_read_policy(read_policy: ReadPolicy) -> Self {
        match read_policy {
            ReadPolicy::LeaseRead => Self::new(None, false).with_wait_timeout(Duration::ZERO),
            ReadPolicy::ReadIndex => Self::new(Some(Duration::ZERO), true),
        }
    }
}

impl From<ReadPolicy> for LinearizerOption {
    fn from(read_policy: ReadPolicy) -> Self {
        Self::from_read_policy(read_policy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_effective_max_quorum_ack_age() {
        let leader_lease = Duration::from_millis(500);
        let cases = [
            (None, leader_lease),
            (Some(Duration::ZERO), Duration::ZERO),
            (Some(Duration::from_millis(300)), Duration::from_millis(300)),
            (Some(leader_lease), leader_lease),
            (Some(Duration::from_millis(700)), leader_lease),
        ];

        for (max_quorum_ack_age, want) in cases {
            let linearizer_option = LinearizerOption::new(max_quorum_ack_age, false);
            let got = linearizer_option.effective_max_quorum_ack_age(leader_lease);
            assert_eq!(want, got);
        }
    }

    /// Unlike the acknowledgement age, the wait timeout is not capped at the leader lease.
    #[test]
    fn test_effective_wait_timeout() {
        let leader_lease = Duration::from_millis(500);
        let cases = [
            (None, leader_lease),
            (Some(Duration::ZERO), Duration::ZERO),
            (Some(Duration::from_millis(300)), Duration::from_millis(300)),
            (Some(Duration::from_millis(700)), Duration::from_millis(700)),
        ];

        for (wait_timeout, want) in cases {
            let mut linearizer_option = LinearizerOption::new(None, false);
            if let Some(wait_timeout) = wait_timeout {
                linearizer_option = linearizer_option.with_wait_timeout(wait_timeout);
            }
            let got = linearizer_option.effective_wait_timeout(leader_lease);
            assert_eq!(want, got);
        }
    }

    #[test]
    fn test_from_read_policy() {
        let lease_read = LinearizerOption::from_read_policy(ReadPolicy::LeaseRead);
        assert_eq!(None, lease_read.max_quorum_ack_age);
        assert!(!lease_read.heartbeat_if_quorum_ack_stale);
        assert_eq!(Some(Duration::ZERO), lease_read.wait_timeout);

        let read_index = LinearizerOption::from_read_policy(ReadPolicy::ReadIndex);
        assert_eq!(Some(Duration::ZERO), read_index.max_quorum_ack_age);
        assert!(read_index.heartbeat_if_quorum_ack_stale);
        assert_eq!(None, read_index.wait_timeout);
    }
}
