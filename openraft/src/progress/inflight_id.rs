use std::fmt;
use std::fmt::Formatter;
use std::ops::Deref;

/// Unique identifier for an inflight replication request.
///
/// Each time the leader sends logs or a snapshot to a follower, it assigns an `InflightId`.
/// When the follower responds, the response carries the same `InflightId`, allowing the leader
/// to correctly match responses to their corresponding requests.
///
/// This prevents stale responses from incorrectly updating the progress state. For example,
/// if a slow response from a previous request arrives after a new request has been sent,
/// the mismatched `InflightId` causes the stale response to be ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InflightId {
    id: u64,
}

impl InflightId {
    pub(crate) fn new(id: u64) -> Self {
        Self { id }
    }
}

impl Deref for InflightId {
    type Target = u64;

    fn deref(&self) -> &Self::Target {
        &self.id
    }
}

impl fmt::Display for InflightId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.id)
    }
}
