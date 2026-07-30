use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use tempfile::TempDir;

use crate::frame::DirFrame;
use crate::frame::DirManifest;
use crate::frame::FORMAT_VERSION;
use crate::frame::FileMeta;
use crate::frame::MAX_CHUNK_SIZE;
use crate::receiver::DirReceiver;
use crate::sender::DirSender;

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

    let mut sender = DirSender::new(dir.path(), 4).unwrap();
    let mut frames = Vec::new();
    while let Some(frame) = sender.next_frame().await.unwrap() {
        frames.push(frame);
    }
    (dir, frames)
}

/// Feed all frames; on success call `finish()`.
async fn receive_all(target: &Path, frames: Vec<DirFrame>) -> io::Result<()> {
    let mut receiver = DirReceiver::new(target.to_path_buf());
    for frame in frames {
        receiver.feed(frame).await?;
    }
    receiver.finish().await
}

/// Assert that receiving `frames` fails with `InvalidData`.
async fn assert_rejected(frames: Vec<DirFrame>) {
    let target = tempfile::tempdir().unwrap();
    let err = receive_all(target.path(), frames).await.unwrap_err();
    assert_eq!(io::ErrorKind::InvalidData, err.kind());
}

fn manifest_frame(files: Vec<FileMeta>) -> DirFrame {
    DirFrame::Manifest(DirManifest {
        format_version: FORMAT_VERSION,
        files,
    })
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

    let mut sender = DirSender::new(source.path(), 4).unwrap();
    let mut frames = Vec::new();
    while let Some(frame) = sender.next_frame().await.unwrap() {
        frames.push(frame);
    }

    receive_all(target.path(), frames).await.unwrap();
    assert_eq!(BTreeMap::new(), dir_contents(target.path()));
}

#[tokio::test]
async fn test_wrong_first_frame() {
    assert_rejected(vec![DirFrame::FileStart {
        name: "a.txt".to_string(),
    }])
    .await;
    assert_rejected(vec![DirFrame::End]).await;
}

#[tokio::test]
async fn test_unsupported_version() {
    assert_rejected(vec![DirFrame::Manifest(DirManifest {
        format_version: FORMAT_VERSION + 1,
        files: vec![],
    })])
    .await;
}

#[tokio::test]
async fn test_manifest_name_validation_precedes_writes() {
    for name in ["../evil", "a/b", "", ".."] {
        let target = tempfile::tempdir().unwrap();
        let mut receiver = DirReceiver::new(target.path().to_path_buf());

        let frame = manifest_frame(vec![FileMeta {
            name: name.to_string(),
            size: 1,
        }]);
        let err = receiver.feed(frame).await.unwrap_err();

        assert_eq!(io::ErrorKind::InvalidData, err.kind(), "name: {name:?}");
        assert_eq!(BTreeMap::new(), dir_contents(target.path()), "no file may be created");
    }
}

#[tokio::test]
async fn test_duplicate_manifest_name() {
    let meta = FileMeta {
        name: "dup".to_string(),
        size: 0,
    };
    assert_rejected(vec![manifest_frame(vec![meta.clone(), meta])]).await;
}

#[tokio::test]
async fn test_file_start_must_follow_manifest_order() {
    let (_source, mut frames) = source_frames().await;
    // Start with the second manifest file instead of the first.
    frames[1] = DirFrame::FileStart {
        name: "b.txt".to_string(),
    };
    assert_rejected(frames).await;
}

#[tokio::test]
async fn test_duplicate_file_start() {
    let (_source, mut frames) = source_frames().await;
    // Frames: [Manifest, Start a, End a, Start b, ...]. Repeat "a.txt" where "b.txt" must start.
    frames[3] = DirFrame::FileStart {
        name: "a.txt".to_string(),
    };
    assert_rejected(frames).await;
}

#[tokio::test]
async fn test_missing_file_end() {
    let (_source, mut frames) = source_frames().await;
    // Remove "a.txt"'s FileEnd so its FileStart is followed by another FileStart.
    frames.remove(2);
    assert_rejected(frames).await;
}

#[tokio::test]
async fn test_end_before_all_manifest_files() {
    let (_source, frames) = source_frames().await;
    // Keep [Manifest, FileStart a, FileEnd a], then End: two manifest files were never sent.
    let mut frames = frames[..3].to_vec();
    frames.push(DirFrame::End);
    assert_rejected(frames).await;
}

#[tokio::test]
async fn test_frames_after_end() {
    let (_source, mut frames) = source_frames().await;
    frames.push(DirFrame::End);
    assert_rejected(frames).await;
}

#[tokio::test]
async fn test_truncated_stream_fails_finish() {
    let (_source, mut frames) = source_frames().await;
    frames.pop();
    assert_rejected(frames).await;
}

#[tokio::test]
async fn test_tampered_chunk() {
    let (_source, mut frames) = source_frames().await;
    let DirFrame::Chunk { data } = &mut frames[4] else {
        panic!("frame 4 must be the first Chunk of b.txt");
    };
    data[0] ^= 0xff;
    assert_rejected(frames).await;
}

#[tokio::test]
async fn test_wrong_checksum() {
    let (_source, mut frames) = source_frames().await;
    let DirFrame::FileEnd { checksum } = &mut frames[2] else {
        panic!("frame 2 must be a.txt's FileEnd");
    };
    *checksum ^= 1;
    assert_rejected(frames).await;
}

#[tokio::test]
async fn test_chunk_beyond_manifest_size() {
    let (_source, mut frames) = source_frames().await;
    frames.insert(6, DirFrame::Chunk {
        data: b"extra".to_vec(),
    });
    assert_rejected(frames).await;
}

#[tokio::test]
async fn test_file_shorter_than_manifest_size() {
    let (_source, mut frames) = source_frames().await;
    // Drop one Chunk of b.txt; its FileEnd then arrives before enough data.
    frames.remove(4);
    assert_rejected(frames).await;
}

#[tokio::test]
async fn test_empty_chunk() {
    let (_source, mut frames) = source_frames().await;
    frames.insert(4, DirFrame::Chunk { data: vec![] });
    assert_rejected(frames).await;
}

#[tokio::test]
async fn test_oversized_chunk() {
    let frames = vec![
        manifest_frame(vec![FileMeta {
            name: "big".to_string(),
            size: (MAX_CHUNK_SIZE + 1) as u64,
        }]),
        DirFrame::FileStart {
            name: "big".to_string(),
        },
        DirFrame::Chunk {
            data: vec![0u8; MAX_CHUNK_SIZE + 1],
        },
    ];
    assert_rejected(frames).await;
}
