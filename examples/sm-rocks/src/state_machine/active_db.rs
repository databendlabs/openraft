//! The writable state-machine database and the `CURRENT` file that names its generation.

use std::fs;
use std::io;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use rocksdb::ColumnFamilyDescriptor;
use rocksdb::DB;
use rocksdb::Options;
use tempfile::NamedTempFile;

use crate::RocksSnapshotData;
use crate::snapshot::GENERATION_DIR_PREFIX;

/// Holds the directory name of the active generation, like RocksDB's own `CURRENT` file
/// names its manifest.
pub(crate) const CURRENT_FILE: &str = "CURRENT";

/// The open writable database of the active generation.
#[derive(Debug)]
pub(crate) struct ActiveDb {
    db: DB,

    /// The generation directory, held by the same handle type a snapshot carries.
    generation: RocksSnapshotData,
}

impl ActiveDb {
    /// Open the generation named by `CURRENT`, or create the first one, then remove every other
    /// generation left behind by an earlier process.
    pub(crate) fn open_or_create(base_dir: &Path) -> io::Result<Arc<Self>> {
        fs::create_dir_all(base_dir)?;
        let active = match read_current(base_dir)? {
            Some(path) => Self::open(RocksSnapshotData::existing(path), &Options::default())?,
            None => Self::create_initial(base_dir)?,
        };
        remove_inactive_generations(base_dir, active.db_path())?;
        Ok(active)
    }

    /// Open a writable copy of the snapshot's checkpoint as a new generation.
    pub(crate) fn from_snapshot(base_dir: &Path, snapshot: &RocksSnapshotData) -> io::Result<Arc<Self>> {
        let checkpoint = snapshot.open_read_only()?;
        let copy = RocksSnapshotData::checkpoint(&checkpoint, base_dir)?;
        Self::open(copy, &Options::default())
    }

    pub(crate) fn db(&self) -> &DB {
        &self.db
    }

    pub(crate) fn db_path(&self) -> &Path {
        self.generation.db_path()
    }

    /// Durably point `CURRENT` at this generation so that a restart opens it.
    pub(crate) fn make_current(&self, base_dir: &Path) -> io::Result<()> {
        // Make the generation's directory entry durable before `CURRENT` names it, then make
        // the `CURRENT` replacement durable. RocksDB has already synced the generation itself.
        sync_directory(base_dir)?;
        write_current(base_dir, self.generation.name())?;
        sync_directory(base_dir)?;

        self.generation.persist();
        Ok(())
    }

    pub(crate) fn mark_for_removal(&self) {
        self.generation.mark_for_removal();
    }

    fn create_initial(base_dir: &Path) -> io::Result<Arc<Self>> {
        let generation = RocksSnapshotData::new_empty(base_dir)?;

        let mut options = Options::default();
        options.create_missing_column_families(true);
        options.create_if_missing(true);

        let active = Self::open(generation, &options)?;
        active.make_current(base_dir)?;
        Ok(active)
    }

    fn open(generation: RocksSnapshotData, options: &Options) -> io::Result<Arc<Self>> {
        let descriptors =
            ["sm_meta", "sm_data"].into_iter().map(|name| ColumnFamilyDescriptor::new(name, Options::default()));
        let db = DB::open_cf_descriptors(options, generation.db_path(), descriptors).map_err(io::Error::other)?;
        Ok(Arc::new(Self { db, generation }))
    }
}

/// Return the generation directory named by `CURRENT`, or `None` when the file does not exist.
fn read_current(base_dir: &Path) -> io::Result<Option<PathBuf>> {
    match fs::read_to_string(base_dir.join(CURRENT_FILE)) {
        Ok(content) => Ok(Some(base_dir.join(content.trim()))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Replace `CURRENT` atomically by renaming a synced temporary file over it.
fn write_current(base_dir: &Path, name: &str) -> io::Result<()> {
    let mut temporary = NamedTempFile::new_in(base_dir)?;
    writeln!(temporary, "{name}")?;
    temporary.as_file_mut().sync_all()?;
    temporary.persist(base_dir.join(CURRENT_FILE)).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    fs::File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn remove_inactive_generations(base_dir: &Path, active_path: &Path) -> io::Result<()> {
    for entry in fs::read_dir(base_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir()
            && entry.file_name().to_string_lossy().starts_with(GENERATION_DIR_PREFIX)
            && entry.path() != active_path
        {
            fs::remove_dir_all(entry.path())?;
        }
    }
    Ok(())
}
