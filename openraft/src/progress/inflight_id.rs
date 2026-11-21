use std::fmt;
use std::fmt::Formatter;
use std::ops::Deref;

/// Identifies one replication operation that the leader started for a target.
///
/// The leader assigns a new `InflightId` each time it starts an [`Inflight`] operation for a
/// target: a probe of a fixed log range, a snapshot, or a `LogsSince` stream of every log after
/// the matching point. A probe ends with its first acknowledgment or conflict, and a snapshot with
/// its acknowledgment. A `LogsSince` stream keeps its ID while several acknowledgments advance its
/// position.
///
/// The ID never goes over the network: AppendEntries and snapshot responses have no such field.
/// The target's local replication worker tags each
/// [`ReplicationProgress`](crate::core::notification::Notification::ReplicationProgress) that it
/// sends to RaftCore with the ID of the operation it is running.
///
/// The leader applies an acknowledgment or a conflict only when its ID matches the current
/// operation, so a late result of an earlier operation cannot update the progress state.
///
/// [`Inflight`]: crate::progress::inflight::Inflight
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
