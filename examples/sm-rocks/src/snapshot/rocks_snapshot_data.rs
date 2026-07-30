//! A handle to one RocksDB generation directory, used as snapshot data.

use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use rocksdb::DB;
use rocksdb::Options;
use rocksdb::checkpoint::Checkpoint;
use tempfile::Builder;

pub(crate) const GENERATION_DIR_PREFIX: &str = "openraft-db-";

/// One RocksDB database directory under the state-machine base directory.
///
/// Every generation is one of these: a checkpoint captured for a snapshot, a checkpoint
/// received from a leader, and the active database that `ActiveDb` opens writable. Clones
/// share the directory, and it is removed when the last clone drops unless it was persisted.
#[derive(Debug, Clone)]
pub struct RocksSnapshotData {
    generation: Arc<GenerationDir>,
}

impl RocksSnapshotData {
    /// Create a new empty generation directory, for example for a transport to receive
    /// checkpoint files into.
    pub fn new_empty(base_dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(base_dir)?;
        let path = Builder::new().prefix(GENERATION_DIR_PREFIX).tempdir_in(base_dir)?.keep();
        Ok(Self::new(path, true))
    }

    /// Return the generation's database directory.
    pub fn db_path(&self) -> &Path {
        &self.generation.path
    }

    /// Create a new generation holding a checkpoint of `db`.
    pub(crate) fn checkpoint(db: &DB, base_dir: &Path) -> io::Result<Self> {
        // RocksDB creates the checkpoint directory itself and refuses an existing one,
        // so reserve a unique name and hand over the path of the removed empty directory.
        let generation = Self::new_empty(base_dir)?;
        fs::remove_dir(generation.db_path())?;

        let checkpoint = Checkpoint::new(db).map_err(io::Error::other)?;
        checkpoint.create_checkpoint(generation.db_path()).map_err(io::Error::other)?;
        Ok(generation)
    }

    /// Wrap the existing active generation; it is kept on drop.
    pub(crate) fn existing(path: PathBuf) -> Self {
        Self::new(path, false)
    }

    /// Open every column family because RocksDB rejects opening only a subset.
    pub(crate) fn open_read_only(&self) -> io::Result<DB> {
        let options = Options::default();
        let column_families = DB::list_cf(&options, self.db_path()).map_err(io::Error::other)?;
        DB::open_cf_for_read_only(&options, self.db_path(), column_families, false).map_err(io::Error::other)
    }

    pub(crate) fn name(&self) -> &str {
        let name = self.db_path().file_name().and_then(|name| name.to_str());
        name.expect("generation names are ASCII")
    }

    /// Keep the directory when the last clone drops.
    pub(crate) fn persist(&self) {
        self.generation.remove_on_drop.store(false, Ordering::Relaxed);
    }

    /// Remove the directory when the last clone drops.
    pub(crate) fn mark_for_removal(&self) {
        self.generation.remove_on_drop.store(true, Ordering::Relaxed);
    }

    fn new(path: PathBuf, remove_on_drop: bool) -> Self {
        Self {
            generation: Arc::new(GenerationDir {
                path,
                remove_on_drop: AtomicBool::new(remove_on_drop),
            }),
        }
    }
}

/// The directory shared by every clone of one [`RocksSnapshotData`].
#[derive(Debug)]
struct GenerationDir {
    path: PathBuf,

    /// Whether the directory is removed when the last clone is dropped.
    remove_on_drop: AtomicBool,
}

impl Drop for GenerationDir {
    fn drop(&mut self) {
        if !self.remove_on_drop.load(Ordering::Relaxed) {
            return;
        }

        if let Err(error) = fs::remove_dir_all(&self.path)
            && error.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!(path = %self.path.display(), %error, "failed to remove RocksDB generation");
        }
    }
}
