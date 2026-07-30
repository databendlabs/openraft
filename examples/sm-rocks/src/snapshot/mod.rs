//! RocksDB checkpoint snapshot types.

mod rocks_snapshot_builder;
mod rocks_snapshot_data;

pub use rocks_snapshot_builder::RocksSnapshotBuilder;
pub(crate) use rocks_snapshot_data::GENERATION_DIR_PREFIX;
pub use rocks_snapshot_data::RocksSnapshotData;
