use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use tempfile::TempDir;
use tokio::fs::File;

use crate::frame::DirFrame;
use crate::frame::DirManifest;
use crate::frame::MAX_CHUNK_SIZE;
use crate::frame::ManifestEntry;
use crate::frame::PROTOCOL_VERSION;
use crate::frame::checksum_digest;
use crate::frame_producer::DirFrameProducer;
use crate::writer::DirWriter;
use crate::writer::State;

/// Read a directory into `name -> contents`, asserting it is flat.
fn dir_contents(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut contents = BTreeMap::new();
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        assert!(entry.file_type().unwrap().is_file());
        let name = entry.file_name().into_string().unwrap();
        contents.insert(name, fs::read(entry.path()).unwrap());
    }
    contents
}

/// Build a source directory and its complete valid frame stream.
async fn source_frames() -> (TempDir, Vec<DirFrame>) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.txt"), b"").unwrap();
    fs::write(dir.path().join("b.txt"), b"1234567").unwrap();
    fs::write(dir.path().join("c.bin"), b"abcdefgh").unwrap();

    let mut producer = DirFrameProducer::new(dir.path(), 4).unwrap();
    let mut frames = Vec::new();
    while let Some(frame) = producer.next_frame().await.unwrap() {
        frames.push(frame);
    }
    (dir, frames)
}

/// Write all frames; on success call `finish()`.
async fn receive_all(target: &Path, frames: Vec<DirFrame>) -> io::Result<()> {
    let mut writer = DirWriter::new(target.to_path_buf());
    for frame in frames {
        writer.write_frame(frame).await?;
    }
    writer.finish().await
}

/// Accept the valid prefix and require the next frame to fail with `InvalidData`.
async fn assert_frame_rejected(prefix: &[DirFrame], frame: DirFrame) {
    let target = tempfile::tempdir().unwrap();
    let mut writer = DirWriter::new(target.path().to_path_buf());
    for valid in prefix {
        writer.write_frame(valid.clone()).await.unwrap();
    }
    let err = writer.write_frame(frame).await.unwrap_err();
    let kind = err.kind();
    assert_eq!(io::ErrorKind::InvalidData, kind);
}

fn manifest_frame(files: Vec<ManifestEntry>) -> DirFrame {
    DirFrame::Manifest(DirManifest {
        format_version: PROTOCOL_VERSION,
        files,
    })
}

fn crc64(data: &[u8]) -> u64 {
    let mut digest = checksum_digest();
    digest.update(data);
    digest.finalize()
}

#[tokio::test]
async fn test_round_trip() {
    let (source, frames) = source_frames().await;
    let target = tempfile::tempdir().unwrap();

    receive_all(target.path(), frames).await.unwrap();

    assert_eq!(dir_contents(source.path()), dir_contents(target.path()));
}

#[tokio::test]
async fn test_round_trip_empty_dir() {
    let source = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();

    let mut producer = DirFrameProducer::new(source.path(), 4).unwrap();
    let mut frames = Vec::new();
    while let Some(frame) = producer.next_frame().await.unwrap() {
        frames.push(frame);
    }

    receive_all(target.path(), frames).await.unwrap();
    assert_eq!(BTreeMap::new(), dir_contents(target.path()));
}

#[tokio::test]
async fn test_wrong_first_frame() {
    let start = DirFrame::FileStart {
        name: "a.txt".to_string(),
    };
    assert_frame_rejected(&[], start).await;
    assert_frame_rejected(&[], DirFrame::End).await;
}

#[tokio::test]
async fn test_unsupported_version() {
    let manifest = DirFrame::Manifest(DirManifest {
        format_version: PROTOCOL_VERSION + 1,
        files: vec![],
    });
    assert_frame_rejected(&[], manifest).await;
}

#[tokio::test]
async fn test_manifest_name_validation_precedes_writes() {
    for name in ["../evil", "a/b", "C:x", "", ".."] {
        let target = tempfile::tempdir().unwrap();
        let mut writer = DirWriter::new(target.path().to_path_buf());

        let frame = manifest_frame(vec![ManifestEntry {
            name: name.to_string(),
            size: 1,
        }]);
        let err = writer.write_frame(frame).await.unwrap_err();

        assert_eq!(io::ErrorKind::InvalidData, err.kind(), "name: {name:?}");
        assert_eq!(BTreeMap::new(), dir_contents(target.path()), "no file may be created");
    }
}

#[tokio::test]
async fn test_duplicate_manifest_name() {
    let meta = ManifestEntry {
        name: "dup".to_string(),
        size: 0,
    };
    let manifest = manifest_frame(vec![meta.clone(), meta]);
    assert_frame_rejected(&[], manifest).await;
}

#[tokio::test]
async fn test_existing_file_fails_the_session() {
    let target = tempfile::tempdir().unwrap();
    fs::write(target.path().join("f"), b"old").unwrap();
    let mut writer = DirWriter::new(target.path().to_path_buf());
    let manifest = manifest_frame(vec![ManifestEntry {
        name: "f".to_string(),
        size: 3,
    }]);
    writer.write_frame(manifest).await.unwrap();

    let err = writer.write_frame(DirFrame::FileStart { name: "f".to_string() }).await.unwrap_err();

    assert_eq!(io::ErrorKind::AlreadyExists, err.kind());
    assert_eq!(b"old".as_slice(), fs::read(target.path().join("f")).unwrap());
}

#[cfg(unix)]
#[tokio::test]
async fn test_symlink_in_target_fails_the_session() {
    let outside = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path().join("f"), target.path().join("f")).unwrap();
    let mut writer = DirWriter::new(target.path().to_path_buf());
    let manifest = manifest_frame(vec![ManifestEntry {
        name: "f".to_string(),
        size: 3,
    }]);
    writer.write_frame(manifest).await.unwrap();

    let err = writer.write_frame(DirFrame::FileStart { name: "f".to_string() }).await.unwrap_err();

    assert_eq!(io::ErrorKind::AlreadyExists, err.kind());
    assert!(
        !outside.path().join("f").exists(),
        "no file may be created through the symlink"
    );
}

#[tokio::test]
async fn test_names_equal_on_the_file_system() {
    let frames = vec![
        manifest_frame(vec![
            ManifestEntry {
                name: "CURRENT".to_string(),
                size: 5,
            },
            ManifestEntry {
                name: "current".to_string(),
                size: 7,
            },
        ]),
        DirFrame::FileStart {
            name: "CURRENT".to_string(),
        },
        DirFrame::Chunk {
            data: b"first".to_vec(),
        },
        DirFrame::FileEnd {
            checksum: crc64(b"first"),
        },
        DirFrame::FileStart {
            name: "current".to_string(),
        },
        DirFrame::Chunk {
            data: b"second!".to_vec(),
        },
        DirFrame::FileEnd {
            checksum: crc64(b"second!"),
        },
        DirFrame::End,
    ];

    // Find out whether the file system of the temporary directories folds case.
    let probe = tempfile::tempdir().unwrap();
    fs::write(probe.path().join("CURRENT"), b"").unwrap();
    let folds_case = probe.path().join("current").exists();

    let target = tempfile::tempdir().unwrap();
    let res = receive_all(target.path(), frames).await;

    let mut expected = BTreeMap::from([("CURRENT".to_string(), b"first".to_vec())]);
    if folds_case {
        // `current` fails at its FileStart and leaves the completed `CURRENT` intact.
        assert_eq!(io::ErrorKind::AlreadyExists, res.unwrap_err().kind());
    } else {
        res.unwrap();
        expected.insert("current".to_string(), b"second!".to_vec());
    }
    assert_eq!(expected, dir_contents(target.path()));
}

#[tokio::test]
async fn test_file_start_must_follow_manifest_order() {
    let (_source, frames) = source_frames().await;
    // Start with the second manifest file instead of the first.
    let start = DirFrame::FileStart {
        name: "b.txt".to_string(),
    };
    assert_frame_rejected(&frames[..1], start).await;
}

#[tokio::test]
async fn test_duplicate_file_start() {
    let (_source, frames) = source_frames().await;
    // Frames: [Manifest, Start a, End a, Start b, ...]. Repeat "a.txt" where "b.txt" must start.
    let start = frames[1].clone();
    assert_frame_rejected(&frames[..3], start).await;
}

#[tokio::test]
async fn test_missing_file_end() {
    let (_source, frames) = source_frames().await;
    // Skip "a.txt"'s FileEnd so its FileStart is followed by another FileStart.
    let start = frames[3].clone();
    assert_frame_rejected(&frames[..2], start).await;
}

#[tokio::test]
async fn test_end_before_all_manifest_files() {
    let (_source, frames) = source_frames().await;
    // Keep [Manifest, FileStart a, FileEnd a], then End: two manifest files were never sent.
    assert_frame_rejected(&frames[..3], DirFrame::End).await;
}

#[tokio::test]
async fn test_frames_after_end() {
    let (_source, frames) = source_frames().await;
    assert_frame_rejected(&frames, DirFrame::End).await;
}

#[tokio::test]
async fn test_truncated_stream_fails_finish() {
    let (_source, mut frames) = source_frames().await;
    frames.pop();
    let target = tempfile::tempdir().unwrap();
    let mut writer = DirWriter::new(target.path().to_path_buf());
    for frame in frames {
        writer.write_frame(frame).await.unwrap();
    }
    let err = writer.finish().await.unwrap_err();
    let kind = err.kind();
    assert_eq!(io::ErrorKind::InvalidData, kind);
}

#[tokio::test]
async fn test_rejected_frame_fails_the_session() {
    let target = tempfile::tempdir().unwrap();
    let mut writer = DirWriter::new(target.path().to_path_buf());
    let manifest = manifest_frame(vec![ManifestEntry {
        name: "a".to_string(),
        size: 1,
    }]);
    writer.write_frame(manifest).await.unwrap();
    let err = writer.write_frame(DirFrame::End).await.unwrap_err();
    assert_eq!(io::ErrorKind::InvalidData, err.kind());

    // The frame that was expected before the failure is no longer accepted.
    let frame = DirFrame::FileStart { name: "a".to_string() };
    let err = writer.write_frame(frame).await.unwrap_err();
    assert_eq!(io::ErrorKind::InvalidData, err.kind());

    let err = writer.finish().await.unwrap_err();
    assert_eq!(io::ErrorKind::InvalidData, err.kind());
}

#[tokio::test]
async fn test_tampered_chunk() {
    let (_source, mut frames) = source_frames().await;
    let DirFrame::Chunk { data } = &mut frames[4] else {
        panic!("frame 4 must be the first Chunk of b.txt");
    };
    data[0] ^= 0xff;
    let file_end = frames[6].clone();
    assert_frame_rejected(&frames[..6], file_end).await;
}

#[tokio::test]
async fn test_wrong_checksum() {
    let (_source, mut frames) = source_frames().await;
    let DirFrame::FileEnd { checksum } = &mut frames[2] else {
        panic!("frame 2 must be a.txt's FileEnd");
    };
    *checksum ^= 1;
    let file_end = frames[2].clone();
    assert_frame_rejected(&frames[..2], file_end).await;
}

#[tokio::test]
async fn test_failed_write_fails_the_session() {
    let target = tempfile::tempdir().unwrap();
    let mut writer = DirWriter::new(target.path().to_path_buf());
    let manifest = manifest_frame(vec![ManifestEntry {
        name: "f".to_string(),
        size: 4,
    }]);
    writer.write_frame(manifest).await.unwrap();
    writer.write_frame(DirFrame::FileStart { name: "f".to_string() }).await.unwrap();

    // Write through a read-only handle, so the chunk's write fails in the background.
    let State::InFile(write) = &mut writer.state else {
        panic!("the writer must be receiving file data");
    };
    write.file = File::open(target.path().join("f")).await.unwrap();

    let checksum = crc64(b"data");

    // Tokio decides whether the failure surfaces at the Chunk or at the FileEnd.
    let res = async {
        writer.write_frame(DirFrame::Chunk { data: b"data".to_vec() }).await?;
        writer.write_frame(DirFrame::FileEnd { checksum }).await
    }
    .await;
    let err = res.unwrap_err();
    assert_ne!(
        io::ErrorKind::InvalidData,
        err.kind(),
        "an I/O error keeps its kind: {err}"
    );

    let err = writer.finish().await.unwrap_err();
    assert_eq!(io::ErrorKind::InvalidData, err.kind());
}

#[tokio::test]
async fn test_chunk_beyond_manifest_size() {
    let (_source, frames) = source_frames().await;
    let extra = DirFrame::Chunk { data: b"extra".to_vec() };
    assert_frame_rejected(&frames[..6], extra).await;
}

#[tokio::test]
async fn test_file_shorter_than_manifest_size() {
    let (_source, frames) = source_frames().await;
    // End b.txt after its first chunk, with the correct checksum for those four bytes.
    let checksum = crc64(b"1234");
    assert_frame_rejected(&frames[..5], DirFrame::FileEnd { checksum }).await;
}

#[tokio::test]
async fn test_empty_chunk() {
    let (_source, frames) = source_frames().await;
    assert_frame_rejected(&frames[..4], DirFrame::Chunk { data: vec![] }).await;
}

#[tokio::test]
async fn test_oversized_chunk() {
    let prefix = [
        manifest_frame(vec![ManifestEntry {
            name: "big".to_string(),
            size: (MAX_CHUNK_SIZE + 1) as u64,
        }]),
        DirFrame::FileStart {
            name: "big".to_string(),
        },
    ];
    let oversized = DirFrame::Chunk {
        data: vec![0u8; MAX_CHUNK_SIZE + 1],
    };
    assert_frame_rejected(&prefix, oversized).await;
}
