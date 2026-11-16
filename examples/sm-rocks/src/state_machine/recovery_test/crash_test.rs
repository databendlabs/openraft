use std::process::Child;
use std::process::Command;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use super::*;

const CHILD_TEST: &str = "state_machine::recovery_test::crash_test::crash_child";
const BOUNDARY_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

#[test]
fn process_interruption_recovers_complete_generations_and_apply_prefixes() {
    TypeConfig::run(async {
        for (point, installed) in [
            ("before_apply_commit", false),
            ("after_apply_commit", true),
            ("candidate_prepared", false),
            ("before_current_replace", false),
            ("before_current_sync", true),
            ("after_current_sync", true),
            ("before_cleanup", true),
            ("after_install", true),
        ] {
            let root = temporary_root();
            let (target, snapshot) = prepare(&root).await;
            drop(snapshot);
            drop(target);
            tracing::info!(point, path = %root.display(), "Kill the child at a named boundary without running destructors");
            {
                let mut child = spawn_child(&root, point);
                stop_at_boundary(&mut child, &root, point);
            }
            recover(&root, installed).await;
            let selected = fs::read_to_string(root.join("target").join(CURRENT_FILE)).unwrap();
            let selected = root.join("target").join(selected.trim());
            let remaining = generations(&root.join("target"));
            assert_eq!(BTreeSet::from([selected]), remaining);
            fs::remove_dir_all(root).unwrap();
        }
    });
}

fn spawn_child(root: &Path, point: &str) -> Child {
    let executable = std::env::current_exe().unwrap();
    let output = fs::File::create(root.join("child.log")).unwrap();
    let errors = output.try_clone().unwrap();
    Command::new(executable)
        .args(["--exact", CHILD_TEST, "--ignored", "--nocapture"])
        .env("OPENRAFT_CRASH_STORE", root.join("target"))
        .env("OPENRAFT_CRASH_POINT", point)
        .stdout(output)
        .stderr(errors)
        .spawn()
        .unwrap()
}

fn stop_at_boundary(child: &mut Child, root: &Path, point: &str) {
    let started = Instant::now();
    let signal = root.join("target").join("boundary-reached");
    while !signal.exists() && started.elapsed() < BOUNDARY_TIMEOUT {
        if child.try_wait().unwrap().is_some() {
            let output = fs::read_to_string(root.join("child.log")).unwrap();
            panic!("child exited before {point}: {output}");
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    child.kill().unwrap();
    let status = child.wait().unwrap();
    assert!(!status.success());
    let reached = fs::read_to_string(signal).expect("child did not signal its boundary before the timeout");
    assert_eq!(point, reached);
}

#[test]
#[ignore = "run in a child process by the interruption test"]
fn crash_child() {
    TypeConfig::run(async {
        let directory = std::env::var_os("OPENRAFT_CRASH_STORE").unwrap();
        let directory = PathBuf::from(directory);
        let point = std::env::var("OPENRAFT_CRASH_POINT").unwrap();
        let mut source = RocksStateMachine::<TypeConfig>::open(directory.parent().unwrap().join("source")).unwrap();
        let mut builder = source.get_snapshot_builder().await;
        let snapshot = builder.build_snapshot().await.unwrap();
        let mut target = RocksStateMachine::<TypeConfig>::open(&directory).unwrap();
        fault::CRASH_ARMED.store(true, Ordering::Relaxed);
        if point.ends_with("apply_commit") {
            apply_entries(&mut target, true).await.unwrap();
        } else {
            target.install_snapshot(&snapshot.meta, snapshot.snapshot).await.unwrap();
            fault::check(&directory, "after_install").unwrap();
        }
        panic!("operation did not reach {point}");
    });
}
