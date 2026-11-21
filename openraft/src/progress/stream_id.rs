use std::fmt;
use std::fmt::Formatter;
use std::ops::Deref;

/// Identifies the leader's progress entry for one replication target, and with it the replication
/// stream that serves that target.
///
/// The leader allocates a new `StreamId` whenever it creates the progress entry of a target: for
/// every target when it becomes leader, and for each target that a membership change adds. The ID
/// stays the same for as long as the entry exists, over many [`InflightId`] operations such as
/// probes, snapshots and `LogsSince` streams.
///
/// Every replication and heartbeat progress notification carries the ID.
/// [`Leader::is_replication_stream_valid`] accepts a notification only when the ID matches the
/// target's current entry. So when a target is removed and added again, a delayed notification
/// from its old stream is ignored before the engine looks at its `InflightId`.
///
/// [`InflightId`]: crate::progress::inflight_id::InflightId
/// [`Leader::is_replication_stream_valid`]: crate::proposer::Leader::is_replication_stream_valid
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StreamId {
    id: u64,
}

impl StreamId {
    pub(crate) fn new(id: u64) -> Self {
        Self { id }
    }
}

impl Deref for StreamId {
    type Target = u64;

    fn deref(&self) -> &Self::Target {
        &self.id
    }
}

impl fmt::Display for StreamId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "StreamId({})", self.id)
    }
}
