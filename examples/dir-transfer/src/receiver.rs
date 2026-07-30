//! Rebuild a directory from an ordered stream of frames and validate it.

use std::collections::BTreeSet;
use std::io;
use std::path::PathBuf;

use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use crate::frame::DirFrame;
use crate::frame::DirManifest;
use crate::frame::FORMAT_VERSION;
use crate::frame::MAX_CHUNK_SIZE;
use crate::frame::checksum_digest;
use crate::frame::invalid_data;
use crate::frame::validate_name;

/// A receiver that writes the frame stream of one session into a target directory.
///
/// Feed every frame in order with [`DirReceiver::feed`], then call [`DirReceiver::finish`]. Every
/// protocol violation fails with [`io::ErrorKind::InvalidData`]; a failed session's directory is
/// discarded by the caller, never reused.
pub struct DirReceiver {
    target_dir: PathBuf,
    manifest: DirManifest,
    state: State,
}

enum State {
    ExpectManifest,
    ExpectFileStart { index: usize },
    InFile(FileWrite),
    ExpectEnd,
    Done,
}

struct FileWrite {
    index: usize,
    file: File,
    digest: crc::Digest<'static, u64>,
    written: u64,
}

impl State {
    fn name(&self) -> &'static str {
        match self {
            State::ExpectManifest => "expecting Manifest",
            State::ExpectFileStart { .. } => "expecting FileStart",
            State::InFile(_) => "receiving file data",
            State::ExpectEnd => "expecting End",
            State::Done => "completed",
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

impl DirReceiver {
    /// Create a receiver that writes into the existing, empty directory `target_dir`.
    pub fn new(target_dir: PathBuf) -> Self {
        Self {
            target_dir,
            manifest: DirManifest {
                format_version: FORMAT_VERSION,
                files: vec![],
            },
            state: State::ExpectManifest,
        }
    }

    /// Consume the next frame of the stream, validating grammar and contents.
    pub async fn feed(&mut self, frame: DirFrame) -> io::Result<()> {
        let state = std::mem::replace(&mut self.state, State::Done);
        self.state = match (state, frame) {
            (State::ExpectManifest, DirFrame::Manifest(manifest)) => self.recv_manifest(manifest)?,
            (State::ExpectFileStart { index }, DirFrame::FileStart { name }) => self.start_file(index, &name).await?,
            (State::InFile(write), DirFrame::Chunk { data }) => self.recv_chunk(write, &data).await?,
            (State::InFile(write), DirFrame::FileEnd { checksum }) => self.end_file(write, checksum).await?,
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
        if !matches!(self.state, State::Done) {
            return Err(invalid_data(format!("stream truncated while {}", self.state.name())));
        }
        std::fs::File::open(&self.target_dir)?.sync_all()
    }

    /// Validate the manifest: version, and every file name before any filesystem write.
    fn recv_manifest(&mut self, manifest: DirManifest) -> io::Result<State> {
        if manifest.format_version != FORMAT_VERSION {
            return Err(invalid_data(format!(
                "unsupported format version {}, expect {FORMAT_VERSION}",
                manifest.format_version
            )));
        }

        let mut names = BTreeSet::new();
        for meta in &manifest.files {
            validate_name(&meta.name)?;
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
        Ok(State::InFile(FileWrite {
            index,
            file,
            digest: checksum_digest(),
            written: 0,
        }))
    }

    /// Append a bounded, size-checked chunk to the current file.
    async fn recv_chunk(&mut self, mut write: FileWrite, data: &[u8]) -> io::Result<State> {
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
        let written = write.written + data.len() as u64;
        if written > size {
            return Err(invalid_data(format!(
                "file data {written} bytes exceeds manifest size {size}"
            )));
        }

        write.file.write_all(data).await?;
        write.digest.update(data);
        write.written = written;
        Ok(State::InFile(write))
    }

    /// Verify size and checksum, then make the completed file durable.
    async fn end_file(&mut self, write: FileWrite, checksum: u64) -> io::Result<State> {
        let meta = &self.manifest.files[write.index];
        if write.written != meta.size {
            return Err(invalid_data(format!(
                "file {:?} ended at {} bytes, manifest size is {}",
                meta.name, write.written, meta.size
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
mod receiver_test;
