//! Snapshot builder backed by a prepared RocksDB checkpoint.

use std::io;

use openraft::RaftSnapshotBuilder;
use openraft::RaftTypeConfig;
use openraft::alias::SnapshotOf;

use super::RocksSnapshotData;

/// A builder holding a checkpoint captured when the builder was created.
///
/// The state machine captures and publishes the checkpoint under `&mut self`, so
/// `build_snapshot` only hands it out.
pub struct RocksSnapshotBuilder<C>
where C: RaftTypeConfig
{
    /// The captured checkpoint, or the error that prevented capturing it.
    prepared: Option<io::Result<SnapshotOf<C, RocksSnapshotData>>>,
}

impl<C> RocksSnapshotBuilder<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(prepared: io::Result<SnapshotOf<C, RocksSnapshotData>>) -> Self {
        Self {
            prepared: Some(prepared),
        }
    }
}

impl<C> RaftSnapshotBuilder<C> for RocksSnapshotBuilder<C>
where C: RaftTypeConfig
{
    type SnapshotData = RocksSnapshotData;

    async fn build_snapshot(&mut self) -> Result<SnapshotOf<C, RocksSnapshotData>, io::Error> {
        self.prepared.take().ok_or_else(|| io::Error::other("snapshot builder has already been consumed"))?
    }
}
