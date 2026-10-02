//! Rebuild a directory from an ordered stream of frames and validate it.

use std::collections::BTreeSet;
use std::io;
use std::path::PathBuf;

use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use crate::frame::DirFrame;
use crate::frame::DirManifest;
use crate::frame::MAX_CHUNK_SIZE;
use crate::frame::PROTOCOL_VERSION;
use crate::frame::checksum_digest;
use crate::frame::invalid_data;
use crate::frame::validate_file_name;

/// A writer that rebuilds and validates one directory from an ordered frame stream.
///
/// Write every frame in order with [`DirWriter::write_frame`], then call [`DirWriter::finish`].
/// Every protocol violation fails with [`io::ErrorKind::InvalidData`]; a failed session's directory
/// is discarded by the caller, never reused.
pub struct DirWriter {
    target_dir: PathBuf,
    manifest: DirManifest,
    state: State,
}

enum State {
    ExpectManifest,
    ExpectFileStart { index: usize },
    InFile(FileWriteState),
    ExpectEnd,
    Done,
    Failed,
}

struct FileWriteState {
    index: usize,
    file: File,
    digest: crc::Digest<'static, u64>,
    bytes_written: u64,
}

impl State {
    fn name(&self) -> &'static str {
        match self {
            State::ExpectManifest => "expecting Manifest",
            State::ExpectFileStart { .. } => "expecting FileStart",
            State::InFile(_) => "receiving file data",
            State::ExpectEnd => "expecting End",
            State::Done => "completed",
            State::Failed => "failed",
        }
    }
}

fn frame_name(frame: &DirFrame) -> &'static str {
    match frame {
        DirFrame::Manifest(_) => "Manifest",
        DirFrame::FileStart { .. } => "FileStart",
        DirFrame::Chunk { .. } => "Chunk",
        DirFrame::FileEnd { .. } => "FileEnd",
        DirFrame::End => "End",
    }
}

impl DirWriter {
    /// Create a writer for the existing, empty directory `target_dir`.
    pub fn new(target_dir: PathBuf) -> Self {
        Self {
            target_dir,
            manifest: DirManifest {
                format_version: PROTOCOL_VERSION,
                files: vec![],
            },
            state: State::ExpectManifest,
        }
    }

    /// Validate and apply the next frame of the transfer.
    pub async fn write_frame(&mut self, frame: DirFrame) -> io::Result<()> {
        // `Failed` stays in place when a branch below returns an error.
        let state = std::mem::replace(&mut self.state, State::Failed);
        self.state = match (state, frame) {
            (State::ExpectManifest, DirFrame::Manifest(manifest)) => self.accept_manifest(manifest)?,
            (State::ExpectFileStart { index }, DirFrame::FileStart { name }) => self.start_file(index, &name).await?,
            (State::InFile(write), DirFrame::Chunk { data }) => self.write_chunk(write, &data).await?,
            (State::InFile(write), DirFrame::FileEnd { checksum }) => self.finish_file(write, checksum).await?,
            (State::ExpectEnd, DirFrame::End) => State::Done,
            (state, frame) => {
                return Err(invalid_data(format!(
                    "unexpected {} frame while {}",
                    frame_name(&frame),
                    state.name()
                )));
            }
        };
        Ok(())
    }

    /// Validate completeness after [`DirFrame::End`] and synchronize the directory.
    pub async fn finish(self) -> io::Result<()> {
        match &self.state {
            State::Done => {}
            State::Failed => return Err(invalid_data("session failed on an earlier frame")),
            state => return Err(invalid_data(format!("stream truncated while {}", state.name()))),
        }
        let dir = File::open(&self.target_dir).await?;
        dir.sync_all().await
    }

    /// Validate the manifest: version, and every file name before any filesystem write.
    fn accept_manifest(&mut self, manifest: DirManifest) -> io::Result<State> {
        if manifest.format_version != PROTOCOL_VERSION {
            return Err(invalid_data(format!(
                "unsupported format version {}, expect {PROTOCOL_VERSION}",
                manifest.format_version
            )));
        }

        let mut names = BTreeSet::new();
        for meta in &manifest.files {
            validate_file_name(&meta.name)?;
            if !names.insert(&meta.name) {
                return Err(invalid_data(format!("duplicate file name {:?}", meta.name)));
            }
        }

        self.manifest = manifest;
        Ok(self.next_file_state(0))
    }

    /// The state that follows the completion of file `index - 1`.
    fn next_file_state(&self, index: usize) -> State {
        if index < self.manifest.files.len() {
            State::ExpectFileStart { index }
        } else {
            State::ExpectEnd
        }
    }

    /// Open the target file after checking the name against the manifest order.
    async fn start_file(&mut self, index: usize, name: &str) -> io::Result<State> {
        let expected = &self.manifest.files[index].name;
        if name != expected {
            return Err(invalid_data(format!(
                "FileStart {name:?} but manifest expects {expected:?}"
            )));
        }

        let file = File::create(self.target_dir.join(name)).await?;
        Ok(State::InFile(FileWriteState {
            index,
            file,
            digest: checksum_digest(),
            bytes_written: 0,
        }))
    }

    /// Append a bounded, size-checked chunk to the current file.
    async fn write_chunk(&mut self, mut write: FileWriteState, data: &[u8]) -> io::Result<State> {
        if data.is_empty() {
            return Err(invalid_data("empty Chunk frame"));
        }
        if data.len() > MAX_CHUNK_SIZE {
            return Err(invalid_data(format!(
                "Chunk of {} bytes exceeds {MAX_CHUNK_SIZE}",
                data.len()
            )));
        }

        let size = self.manifest.files[write.index].size;
        let bytes_written = write.bytes_written + data.len() as u64;
        if bytes_written > size {
            return Err(invalid_data(format!(
                "file data {bytes_written} bytes exceeds manifest size {size}"
            )));
        }

        write.file.write_all(data).await?;
        write.digest.update(data);
        write.bytes_written = bytes_written;
        Ok(State::InFile(write))
    }

    /// Verify size and checksum, then make the completed file durable.
    async fn finish_file(&mut self, write: FileWriteState, checksum: u64) -> io::Result<State> {
        let meta = &self.manifest.files[write.index];
        if write.bytes_written != meta.size {
            return Err(invalid_data(format!(
                "file {:?} ended at {} bytes, manifest size is {}",
                meta.name, write.bytes_written, meta.size
            )));
        }

        let actual = write.digest.finalize();
        if actual != checksum {
            return Err(invalid_data(format!(
                "file {:?} checksum {actual:#018x} does not match {checksum:#018x}",
                meta.name
            )));
        }

        write.file.sync_all().await?;
        Ok(self.next_file_state(write.index + 1))
    }
}

#[cfg(test)]
mod writer_test;
