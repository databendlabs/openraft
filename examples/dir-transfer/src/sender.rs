//! Turn a flat directory of immutable files into an ordered stream of frames.

use std::io;
use std::path::Path;
use std::path::PathBuf;

use tokio::fs::File;
use tokio::io::AsyncReadExt;

use crate::frame::DirFrame;
use crate::frame::DirManifest;
use crate::frame::FORMAT_VERSION;
use crate::frame::FileMeta;
use crate::frame::MAX_CHUNK_SIZE;
use crate::frame::checksum_digest;
use crate::frame::invalid_data;
use crate::frame::validate_name;

/// A pull-based sender that emits the frame stream of one directory.
///
/// The transport drives pacing by awaiting [`DirSender::next_frame`], which yields natural
/// backpressure. The directory must stay immutable while the sender runs; a file whose size
/// changes mid-transfer fails the session.
pub struct DirSender {
    dir: PathBuf,
    chunk_size: usize,
    manifest: DirManifest,
    state: State,
}

enum State {
    SendManifest,
    StartFile { index: usize },
    SendFile(FileProgress),
    SendEnd,
    Done,
}

struct FileProgress {
    index: usize,
    file: File,
    digest: crc::Digest<'static, u64>,
    sent: u64,
}

impl DirSender {
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
            validate_name(&name)?;

            files.push(FileMeta {
                name,
                size: entry.metadata()?.len(),
            });
        }
        files.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(Self {
            dir: dir.to_path_buf(),
            chunk_size,
            manifest: DirManifest {
                format_version: FORMAT_VERSION,
                files,
            },
            state: State::SendManifest,
        })
    }

    /// Return the next frame of the stream, or `None` after [`DirFrame::End`] has been emitted.
    pub async fn next_frame(&mut self) -> io::Result<Option<DirFrame>> {
        match std::mem::replace(&mut self.state, State::Done) {
            State::SendManifest => {
                self.state = self.next_file_state(0);
                Ok(Some(DirFrame::Manifest(self.manifest.clone())))
            }
            State::StartFile { index } => {
                let meta = &self.manifest.files[index];
                let file = File::open(self.dir.join(&meta.name)).await?;
                self.state = State::SendFile(FileProgress {
                    index,
                    file,
                    digest: checksum_digest(),
                    sent: 0,
                });
                Ok(Some(DirFrame::FileStart {
                    name: meta.name.clone(),
                }))
            }
            State::SendFile(progress) => self.send_file(progress).await.map(Some),
            State::SendEnd => Ok(Some(DirFrame::End)),
            State::Done => Ok(None),
        }
    }

    /// The state that follows the completion of file `index - 1`.
    fn next_file_state(&self, index: usize) -> State {
        if index < self.manifest.files.len() {
            State::StartFile { index }
        } else {
            State::SendEnd
        }
    }

    /// Emit the next `Chunk` of the current file, or its `FileEnd` at end of file.
    async fn send_file(&mut self, mut progress: FileProgress) -> io::Result<DirFrame> {
        let data = read_chunk(&mut progress.file, self.chunk_size).await?;
        let size = self.manifest.files[progress.index].size;

        if data.is_empty() {
            if progress.sent != size {
                return Err(invalid_data(format!(
                    "file shrank during transfer: {} < {size}",
                    progress.sent
                )));
            }
            self.state = self.next_file_state(progress.index + 1);
            return Ok(DirFrame::FileEnd {
                checksum: progress.digest.finalize(),
            });
        }

        progress.digest.update(&data);
        progress.sent += data.len() as u64;
        if progress.sent > size {
            return Err(invalid_data(format!(
                "file grew during transfer: {} > {size}",
                progress.sent
            )));
        }

        self.state = State::SendFile(progress);
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

    use super::DirSender;
    use crate::frame::DirFrame;
    use crate::frame::DirManifest;
    use crate::frame::FORMAT_VERSION;
    use crate::frame::FileMeta;
    use crate::frame::MAX_CHUNK_SIZE;
    use crate::frame::checksum_digest;

    fn crc64(data: &[u8]) -> u64 {
        let mut digest = checksum_digest();
        digest.update(data);
        digest.finalize()
    }

    async fn collect_frames(sender: &mut DirSender) -> io::Result<Vec<DirFrame>> {
        let mut frames = Vec::new();
        while let Some(frame) = sender.next_frame().await? {
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

        let mut sender = DirSender::new(dir.path(), 4).unwrap();
        let frames = collect_frames(&mut sender).await.unwrap();

        let expected = vec![
            DirFrame::Manifest(DirManifest {
                format_version: FORMAT_VERSION,
                files: vec![
                    FileMeta {
                        name: "a.txt".to_string(),
                        size: 0,
                    },
                    FileMeta {
                        name: "b.txt".to_string(),
                        size: 7,
                    },
                    FileMeta {
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
        assert!(sender.next_frame().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_empty_dir() {
        let dir = tempfile::tempdir().unwrap();

        let mut sender = DirSender::new(dir.path(), 4).unwrap();
        let frames = collect_frames(&mut sender).await.unwrap();

        let expected = vec![
            DirFrame::Manifest(DirManifest {
                format_version: FORMAT_VERSION,
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
            let err = DirSender::new(dir.path(), chunk_size).err().unwrap();
            assert_eq!(io::ErrorKind::InvalidData, err.kind());
        }
    }

    #[test]
    fn test_rejects_subdirectory() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();

        let err = DirSender::new(dir.path(), 4).err().unwrap();
        assert_eq!(io::ErrorKind::InvalidData, err.kind());
    }

    #[cfg(unix)]
    #[test]
    fn test_rejects_symlink() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("real"), b"x").unwrap();
        std::os::unix::fs::symlink(dir.path().join("real"), dir.path().join("link")).unwrap();

        let err = DirSender::new(dir.path(), 4).err().unwrap();
        assert_eq!(io::ErrorKind::InvalidData, err.kind());
    }

    #[tokio::test]
    async fn test_file_size_drift_fails() {
        // The file shrinks after the manifest is built.
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("f"), b"12345678").unwrap();
        let mut sender = DirSender::new(dir.path(), 4).unwrap();
        fs::write(dir.path().join("f"), b"12").unwrap();

        let err = collect_frames(&mut sender).await.unwrap_err();
        assert_eq!(io::ErrorKind::InvalidData, err.kind());

        // The file grows after the manifest is built.
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("f"), b"12").unwrap();
        let mut sender = DirSender::new(dir.path(), 4).unwrap();
        fs::write(dir.path().join("f"), b"12345678").unwrap();

        let err = collect_frames(&mut sender).await.unwrap_err();
        assert_eq!(io::ErrorKind::InvalidData, err.kind());
    }
}
