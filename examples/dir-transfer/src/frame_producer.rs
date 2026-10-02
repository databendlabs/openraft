//! Produce ordered transfer frames from a flat directory of immutable files.

use std::io;
use std::path::Path;
use std::path::PathBuf;

use tokio::fs::File;
use tokio::io::AsyncReadExt;

use crate::frame::DirFrame;
use crate::frame::DirManifest;
use crate::frame::MAX_CHUNK_SIZE;
use crate::frame::ManifestEntry;
use crate::frame::PROTOCOL_VERSION;
use crate::frame::checksum_digest;
use crate::frame::invalid_data;
use crate::frame::validate_file_name;

/// An asynchronous frame producer for one directory of immutable files.
///
/// The transport drives pacing by awaiting [`DirFrameProducer::next_frame`], which yields natural
/// backpressure. The directory must stay immutable while frames are produced; a file whose
/// size changes mid-transfer fails the session.
pub struct DirFrameProducer {
    dir: PathBuf,
    chunk_size: usize,
    manifest: DirManifest,
    state: State,
}

enum State {
    EmitManifest,
    StartFile { index: usize },
    ReadFile(FileReadState),
    EmitEnd,
    Done,
    Failed,
}

struct FileReadState {
    index: usize,
    file: File,
    digest: crc::Digest<'static, u64>,
    bytes_read: u64,
}

impl DirFrameProducer {
    /// Enumerate `dir` and build the manifest.
    ///
    /// The directory must be flat: every entry a regular file with a valid name. Files are
    /// transferred in name order. `chunk_size` must be in `1..=MAX_CHUNK_SIZE`.
    pub fn new(dir: &Path, chunk_size: usize) -> io::Result<Self> {
        if chunk_size == 0 || chunk_size > MAX_CHUNK_SIZE {
            return Err(invalid_data(format!(
                "chunk_size {chunk_size} not in 1..={MAX_CHUNK_SIZE}"
            )));
        }

        let mut files = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                return Err(invalid_data(format!("{:?} is not a regular file", entry.path())));
            }

            let name = entry
                .file_name()
                .into_string()
                .map_err(|name| invalid_data(format!("file name {name:?} is not UTF-8")))?;
            validate_file_name(&name)?;

            files.push(ManifestEntry {
                name,
                size: entry.metadata()?.len(),
            });
        }
        files.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(Self {
            dir: dir.to_path_buf(),
            chunk_size,
            manifest: DirManifest {
                format_version: PROTOCOL_VERSION,
                files,
            },
            state: State::EmitManifest,
        })
    }

    /// Return the next frame, or `None` after [`DirFrame::End`] has been emitted.
    pub async fn next_frame(&mut self) -> io::Result<Option<DirFrame>> {
        // `Failed` stays in place when a branch below returns an error.
        let state = std::mem::replace(&mut self.state, State::Failed);
        match state {
            State::EmitManifest => {
                self.state = self.next_file_state(0);
                Ok(Some(DirFrame::Manifest(self.manifest.clone())))
            }
            State::StartFile { index } => {
                let meta = &self.manifest.files[index];
                let file = File::open(self.dir.join(&meta.name)).await?;
                self.state = State::ReadFile(FileReadState {
                    index,
                    file,
                    digest: checksum_digest(),
                    bytes_read: 0,
                });
                Ok(Some(DirFrame::FileStart {
                    name: meta.name.clone(),
                }))
            }
            State::ReadFile(progress) => self.next_file_frame(progress).await.map(Some),
            State::EmitEnd => {
                self.state = State::Done;
                Ok(Some(DirFrame::End))
            }
            State::Done => {
                self.state = State::Done;
                Ok(None)
            }
            State::Failed => Err(invalid_data("session failed on an earlier frame")),
        }
    }

    /// The state that follows the completion of file `index - 1`.
    fn next_file_state(&self, index: usize) -> State {
        if index < self.manifest.files.len() {
            State::StartFile { index }
        } else {
            State::EmitEnd
        }
    }

    /// Emit the next `Chunk` of the current file, or its `FileEnd` at end of file.
    async fn next_file_frame(&mut self, mut progress: FileReadState) -> io::Result<DirFrame> {
        let data = read_chunk(&mut progress.file, self.chunk_size).await?;
        let size = self.manifest.files[progress.index].size;

        if data.is_empty() {
            if progress.bytes_read != size {
                return Err(invalid_data(format!(
                    "file shrank during transfer: {} < {size}",
                    progress.bytes_read
                )));
            }
            self.state = self.next_file_state(progress.index + 1);
            return Ok(DirFrame::FileEnd {
                checksum: progress.digest.finalize(),
            });
        }

        progress.digest.update(&data);
        progress.bytes_read += data.len() as u64;
        if progress.bytes_read > size {
            return Err(invalid_data(format!(
                "file grew during transfer: {} > {size}",
                progress.bytes_read
            )));
        }

        self.state = State::ReadFile(progress);
        Ok(DirFrame::Chunk { data })
    }
}

/// Read up to `chunk_size` bytes; a short result means end of file.
async fn read_chunk(file: &mut File, chunk_size: usize) -> io::Result<Vec<u8>> {
    let mut data = vec![0u8; chunk_size];
    let mut filled = 0;
    while filled < chunk_size {
        let n = file.read(&mut data[filled..]).await?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    data.truncate(filled);
    Ok(data)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;

    use super::DirFrameProducer;
    use crate::frame::DirFrame;
    use crate::frame::DirManifest;
    use crate::frame::MAX_CHUNK_SIZE;
    use crate::frame::ManifestEntry;
    use crate::frame::PROTOCOL_VERSION;
    use crate::frame::checksum_digest;

    fn crc64(data: &[u8]) -> u64 {
        let mut digest = checksum_digest();
        digest.update(data);
        digest.finalize()
    }

    async fn collect_frames(producer: &mut DirFrameProducer) -> io::Result<Vec<DirFrame>> {
        let mut frames = Vec::new();
        while let Some(frame) = producer.next_frame().await? {
            frames.push(frame);
        }
        Ok(frames)
    }

    #[tokio::test]
    async fn test_frame_sequence() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("b.txt"), b"1234567").unwrap();
        fs::write(dir.path().join("a.txt"), b"").unwrap();
        fs::write(dir.path().join("c.bin"), b"abcdefgh").unwrap();

        let mut producer = DirFrameProducer::new(dir.path(), 4).unwrap();
        let frames = collect_frames(&mut producer).await.unwrap();

        let expected = vec![
            DirFrame::Manifest(DirManifest {
                format_version: PROTOCOL_VERSION,
                files: vec![
                    ManifestEntry {
                        name: "a.txt".to_string(),
                        size: 0,
                    },
                    ManifestEntry {
                        name: "b.txt".to_string(),
                        size: 7,
                    },
                    ManifestEntry {
                        name: "c.bin".to_string(),
                        size: 8,
                    },
                ],
            }),
            // Zero-byte file: no Chunk at all.
            DirFrame::FileStart {
                name: "a.txt".to_string(),
            },
            DirFrame::FileEnd { checksum: crc64(b"") },
            DirFrame::FileStart {
                name: "b.txt".to_string(),
            },
            DirFrame::Chunk { data: b"1234".to_vec() },
            DirFrame::Chunk { data: b"567".to_vec() },
            DirFrame::FileEnd {
                checksum: crc64(b"1234567"),
            },
            // Size is an exact multiple of the chunk size.
            DirFrame::FileStart {
                name: "c.bin".to_string(),
            },
            DirFrame::Chunk { data: b"abcd".to_vec() },
            DirFrame::Chunk { data: b"efgh".to_vec() },
            DirFrame::FileEnd {
                checksum: crc64(b"abcdefgh"),
            },
            DirFrame::End,
        ];
        assert_eq!(expected, frames);

        // The stream stays exhausted.
        assert!(producer.next_frame().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_empty_dir() {
        let dir = tempfile::tempdir().unwrap();

        let mut producer = DirFrameProducer::new(dir.path(), 4).unwrap();
        let frames = collect_frames(&mut producer).await.unwrap();

        let expected = vec![
            DirFrame::Manifest(DirManifest {
                format_version: PROTOCOL_VERSION,
                files: vec![],
            }),
            DirFrame::End,
        ];
        assert_eq!(expected, frames);
    }

    #[test]
    fn test_invalid_chunk_size() {
        let dir = tempfile::tempdir().unwrap();

        for chunk_size in [0, MAX_CHUNK_SIZE + 1] {
            let err = DirFrameProducer::new(dir.path(), chunk_size).err().unwrap();
            assert_eq!(io::ErrorKind::InvalidData, err.kind());
        }
    }

    #[test]
    fn test_rejects_subdirectory() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();

        let err = DirFrameProducer::new(dir.path(), 4).err().unwrap();
        assert_eq!(io::ErrorKind::InvalidData, err.kind());
    }

    #[cfg(unix)]
    #[test]
    fn test_rejects_symlink() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("real"), b"x").unwrap();
        std::os::unix::fs::symlink(dir.path().join("real"), dir.path().join("link")).unwrap();

        let err = DirFrameProducer::new(dir.path(), 4).err().unwrap();
        assert_eq!(io::ErrorKind::InvalidData, err.kind());
    }

    #[tokio::test]
    async fn test_file_size_drift_fails() {
        // The file shrinks after the manifest is built.
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("f"), b"12345678").unwrap();
        let mut producer = DirFrameProducer::new(dir.path(), 4).unwrap();
        fs::write(dir.path().join("f"), b"12").unwrap();

        let err = collect_frames(&mut producer).await.unwrap_err();
        assert_eq!(io::ErrorKind::InvalidData, err.kind());

        // The failure is sticky: the stream does not end cleanly afterwards.
        let err = producer.next_frame().await.unwrap_err();
        assert_eq!(io::ErrorKind::InvalidData, err.kind());

        // The file grows after the manifest is built.
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("f"), b"12").unwrap();
        let mut producer = DirFrameProducer::new(dir.path(), 4).unwrap();
        fs::write(dir.path().join("f"), b"12345678").unwrap();

        let err = collect_frames(&mut producer).await.unwrap_err();
        assert_eq!(io::ErrorKind::InvalidData, err.kind());
    }
}
