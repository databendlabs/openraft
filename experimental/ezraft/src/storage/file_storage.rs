//! A file-based [`EzStorage`]

use std::io;
use std::io::Cursor;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::path::PathBuf;

use async_trait::async_trait;
use tokio::fs;

use crate::app::EzApp;
use crate::entry::EzEntry;
use crate::meta::EzMeta;
use crate::snapshot::EzSnapshot;
use crate::snapshot::EzSnapshotMeta;
use crate::storage::EzStorage;
use crate::storage::Loaded;
use crate::storage::Persist;

/// Keeps one node's Raft state as JSON files under a directory
///
/// Enough to get a cluster running without writing storage code first, and shaped to be read:
/// every file is JSON, one log entry per file, so `ls` and `cat` show what Raft persisted. It
/// is not the storage a deployment keeps.
///
/// # Layout
///
/// ```text
/// <base_dir>/meta.json        Raft metadata (node id, vote, log positions)
/// <base_dir>/logs/log-<idx>   one log entry per file
/// <base_dir>/snapshot.meta    snapshot metadata
/// <base_dir>/snapshot.data    serialized application state
/// ```
///
/// # Write your own when these matter
///
/// - **Durability.** Writes are not `fsync`ed, so a machine crash can lose a vote or a log entry
///   this node already acknowledged - the one assumption [`EzStorage`] cannot do without. Anything
///   beyond a demo needs storage that returns from `persist` only once the bytes would survive a
///   power loss.
/// - **Log size.** One file and one directory entry per entry costs an inode and a round of
///   metadata I/O per write. A log-structured file or an embedded store amortizes both.
/// - **Encoding.** JSON is readable and slow, and it cannot represent every Rust type a request
///   might hold.
///
/// [`EzStorage`] is three methods, and this file is the worked example of implementing them.
pub struct FileStorage {
    base_dir: PathBuf,
}

impl FileStorage {
    /// Open (and create if missing) the directory holding this node's state
    ///
    /// One directory per node: two nodes sharing it would overwrite each other's log.
    pub async fn new(base_dir: impl Into<PathBuf>) -> Result<Self, io::Error> {
        let base_dir = base_dir.into();
        fs::create_dir_all(&base_dir).await?;
        Ok(Self { base_dir })
    }

    fn meta_path(&self) -> PathBuf {
        self.base_dir.join("meta.json")
    }

    fn logs_dir(&self) -> PathBuf {
        self.base_dir.join("logs")
    }

    fn log_path(&self, index: u64) -> PathBuf {
        self.logs_dir().join(format!("log-{}", index))
    }

    fn snapshot_meta_path(&self) -> PathBuf {
        self.base_dir.join("snapshot.meta")
    }

    fn snapshot_data_path(&self) -> PathBuf {
        self.base_dir.join("snapshot.data")
    }

    /// Delete every log file whose index satisfies `remove`
    async fn remove_logs(&self, remove: impl Fn(u64) -> bool) -> Result<(), io::Error> {
        let mut dir = match fs::read_dir(self.logs_dir()).await {
            Ok(dir) => dir,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };

        while let Some(entry) = dir.next_entry().await? {
            let name = entry.file_name();
            let index = name.to_str().and_then(|n| n.strip_prefix("log-")).and_then(|n| n.parse::<u64>().ok());

            let Some(index) = index else {
                return Err(io::Error::other(format!("unexpected file in log dir: {:?}", name)));
            };

            if remove(index) {
                fs::remove_file(entry.path()).await?;
            }
        }

        Ok(())
    }
}

#[async_trait]
impl<T> EzStorage<T> for FileStorage
where T: EzApp
{
    async fn load(&mut self) -> Result<Loaded, io::Error> {
        // Load meta (use default if not found)
        let meta = match fs::read(&self.meta_path()).await {
            Ok(data) => serde_json::from_slice(&data)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => EzMeta::default(),
            Err(e) => return Err(e),
        };

        // Load snapshot (optional)
        let snapshot = match fs::read(&self.snapshot_meta_path()).await {
            Ok(meta_data) => {
                let snap_meta: EzSnapshotMeta = serde_json::from_slice(&meta_data)?;
                let data = fs::read(&self.snapshot_data_path()).await?;
                Some(EzSnapshot {
                    meta: snap_meta,
                    snapshot: Cursor::new(data),
                })
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };

        Ok(Loaded { meta, snapshot })
    }

    async fn persist(&mut self, op: Persist<T>) -> Result<(), io::Error> {
        match op {
            Persist::Meta(meta) => {
                fs::write(&self.meta_path(), serde_json::to_vec_pretty(&meta)?).await?;
            }
            Persist::LogEntry(entry) => {
                fs::create_dir_all(&self.logs_dir()).await?;
                let (_, index) = entry.log_id;
                fs::write(self.log_path(index), serde_json::to_vec(&entry)?).await?;
            }
            Persist::Snapshot(snapshot) => {
                fs::write(&self.snapshot_meta_path(), serde_json::to_vec(&snapshot.meta)?).await?;
                // Extract data from cursor
                let mut cursor = snapshot.snapshot;
                cursor.seek(SeekFrom::Start(0))?;
                let mut data = Vec::new();
                cursor.read_to_end(&mut data)?;
                fs::write(&self.snapshot_data_path(), data).await?;
            }
            Persist::DeleteLogs { from, to } => self.remove_logs(|index| (from..to).contains(&index)).await?,
        }
        Ok(())
    }

    async fn read_logs(&mut self, start: u64, end: u64) -> Result<Vec<EzEntry<T>>, io::Error> {
        let mut logs = Vec::new();

        // Every index in the range must be there: a gap handed back to Raft would look like a
        // shorter log rather than the missing entry it is.
        for index in start..end {
            let data = fs::read(&self.log_path(index)).await?;
            logs.push(serde_json::from_slice(&data)?);
        }

        Ok(logs)
    }
}
