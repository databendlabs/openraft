//! This test runs in its own process because it changes the working directory.

use std::env;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use dir_transfer::DirFrameProducer;
use dir_transfer::DirWriter;

/// The sorted names of the entries in `dir`.
fn entry_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> =
        fs::read_dir(dir).unwrap().map(|entry| entry.unwrap().file_name().into_string().unwrap()).collect();
    names.sort();
    names
}

#[tokio::test]
async fn target_survives_a_working_directory_change() {
    let original_dir = env::current_dir().unwrap();
    let base = tempfile::tempdir().unwrap();
    for dir in ["source", "a/target", "b/target"] {
        fs::create_dir_all(base.path().join(dir)).unwrap();
    }
    fs::write(base.path().join("source/f1"), b"one").unwrap();
    fs::write(base.path().join("source/f2"), b"two").unwrap();
    let mut producer = DirFrameProducer::new(&base.path().join("source"), 4).unwrap();

    env::set_current_dir(base.path().join("a")).unwrap();
    let mut writer = DirWriter::new(PathBuf::from("target"));
    let manifest = producer.next_frame().await.unwrap().unwrap();
    writer.write_frame(manifest).await.unwrap();

    // Move the working directory after the session has started.
    env::set_current_dir(base.path().join("b")).unwrap();
    while let Some(frame) = producer.next_frame().await.unwrap() {
        writer.write_frame(frame).await.unwrap();
    }
    writer.finish().await.unwrap();
    env::set_current_dir(original_dir).unwrap();

    assert_eq!(vec!["f1", "f2"], entry_names(&base.path().join("a/target")));
    assert_eq!(Vec::<String>::new(), entry_names(&base.path().join("b/target")));
}
