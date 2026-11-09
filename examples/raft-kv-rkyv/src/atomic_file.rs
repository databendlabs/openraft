use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use fs2::FileExt;

/// Unix atomic replacement with a single-writer lock and an explicit flush boundary.
/// The caller supplies an existing, durably created directory on a local filesystem.
pub(crate) struct AtomicFile {
    path: PathBuf,
    directory: File,
    _lock: File,
    failed: bool,
    #[cfg(test)]
    pub(crate) fail_at: Option<&'static str>,
}

impl AtomicFile {
    pub(crate) fn open(directory: &Path) -> io::Result<Self> {
        let directory_path = directory.canonicalize()?;
        let directory = File::open(&directory_path)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory_path.join("LOCK"))?;
        FileExt::try_lock_exclusive(&lock)?;
        // A previous process may have died after rename but before directory sync. Stabilize
        // whichever generation is visible before Raft can treat recovered votes as durable.
        directory.sync_all()?;
        Ok(Self {
            path: directory_path.join("log-state"),
            directory,
            _lock: lock,
            failed: false,
            #[cfg(test)]
            fail_at: None,
        })
    }

    pub(crate) fn check(&self) -> io::Result<()> {
        if self.failed {
            return Err(io::Error::other(
                "publication failed; drop all store handles and reopen",
            ));
        }
        Ok(())
    }

    pub(crate) fn read(&self, limit: u64) -> io::Result<Option<Vec<u8>>> {
        self.check()?;
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let mut bytes = Vec::new();
        file.take(limit + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > limit {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "disk image too large"));
        }
        Ok(Some(bytes))
    }

    pub(crate) fn publish(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.check()?;
        let result = self.replace(bytes);
        // A failure after rename leaves an uncertain outcome. Never continue using a stale
        // in-memory image, and never acknowledge an operation whose directory sync failed.
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn replace(&self, bytes: &[u8]) -> io::Result<()> {
        let mut next =
            OpenOptions::new().create(true).truncate(true).write(true).open(self.path.with_extension("next"))?;
        // Split writes give the failure tests a genuinely incomplete temporary file.
        let middle = bytes.len() / 2;
        next.write_all(&bytes[..middle])?;
        self.checkpoint("partial_write")?;
        next.write_all(&bytes[middle..])?;
        self.checkpoint("before_file_sync")?;
        next.sync_all()?;
        self.checkpoint("after_file_sync")?;
        std::fs::rename(self.path.with_extension("next"), &self.path)?;
        self.checkpoint("after_rename")?;
        self.directory.sync_all()?;
        self.checkpoint("after_directory_sync")?;
        Ok(())
    }

    fn checkpoint(&self, _stage: &'static str) -> io::Result<()> {
        #[cfg(test)]
        if self.fail_at == Some(_stage) {
            return Err(io::Error::other(format!("injected publication failure: {_stage}")));
        }
        Ok(())
    }
}
