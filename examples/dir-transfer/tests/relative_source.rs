//! This test runs in its own process because it changes the working directory.

use std::env;
use std::fs;
use std::path::Path;

use dir_transfer::DirFrameProducer;

#[tokio::test]
async fn source_survives_a_working_directory_change() {
    let original_dir = env::current_dir().unwrap();
    let source = tempfile::tempdir().unwrap();
    fs::write(source.path().join("file"), b"original").unwrap();
    let mut expected = DirFrameProducer::new(source.path(), 4).unwrap();

    env::set_current_dir(source.path()).unwrap();
    let mut producer = DirFrameProducer::new(Path::new("."), 4).unwrap();
    env::set_current_dir(original_dir).unwrap();

    loop {
        let expected_frame = expected.next_frame().await.unwrap();
        let frame = producer.next_frame().await.unwrap();
        assert_eq!(expected_frame, frame);
        if frame.is_none() {
            break;
        }
    }
}
