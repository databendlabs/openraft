//! Snapshot building for [`FjallStateMachine`].

use std::io;
use std::io::Cursor;

use openraft::RaftSnapshotBuilder;
use openraft::RaftTypeConfig;
use openraft::alias::SnapshotOf;
use openraft::type_config::TypeConfigExt;

use crate::FjallStateMachine;

/// Builds a snapshot from a read view of a [`FjallStateMachine`].
///
/// The view is a fjall snapshot taken when the builder is created, so entries applied while the
/// snapshot is being built do not leak into it. fjall keeps the data the view refers to until the
/// builder is dropped.
pub struct FjallSnapshotBuilder<C>
where C: RaftTypeConfig
{
    /// The state machine whose current snapshot is replaced by the built one.
    sm: FjallStateMachine<C>,

    /// The state machine data when the builder was created.
    view: fjall::Snapshot,
}

impl<C> FjallSnapshotBuilder<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(sm: FjallStateMachine<C>, view: fjall::Snapshot) -> Self {
        Self { sm, view }
    }
}

impl<C> RaftSnapshotBuilder<C> for FjallSnapshotBuilder<C>
where C: RaftTypeConfig
{
    type SnapshotData = Cursor<Vec<u8>>;

    async fn build_snapshot(&mut self) -> Result<SnapshotOf<C, Self::SnapshotData>, io::Error> {
        let sm = self.sm.clone();
        let view = self.view.clone();

        // Scanning and storing the whole state is blocking IO: keep it off the async runtime.
        C::spawn_blocking(move || {
            let meta = sm.read_applied(&view)?;
            let data = sm.encode_data(&view)?;
            sm.save_built_snapshot(&meta, &data)?;

            Ok(SnapshotOf::<C, Self::SnapshotData> {
                meta,
                snapshot: Cursor::new(data),
            })
        })
        .await?
    }
}
