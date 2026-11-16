//! Test-only failures scoped to one store directory, including blocking worker threads.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

static FAILURES: Mutex<BTreeMap<PathBuf, &'static str>> = Mutex::new(BTreeMap::new());
pub(super) static CRASH_ARMED: AtomicBool = AtomicBool::new(false);

pub(super) struct Failure(PathBuf);

impl Failure {
    pub(super) fn new(base: &Path, point: &'static str) -> Self {
        let base = fs::canonicalize(base).unwrap();
        let previous = FAILURES.lock().unwrap().insert(base.clone(), point);
        assert_eq!(None, previous);
        Self(base)
    }
}

impl Drop for Failure {
    fn drop(&mut self) {
        FAILURES.lock().unwrap().remove(&self.0);
    }
}

pub(crate) fn check(base: &Path, point: &str) -> io::Result<()> {
    let failure = FAILURES.lock().unwrap().get(base).copied();
    if failure == Some(point) {
        return Err(io::Error::other(format!("injected failure: {point}")));
    }
    let selected = std::env::var("OPENRAFT_CRASH_POINT").ok();
    let directory = std::env::var_os("OPENRAFT_CRASH_STORE").map(PathBuf::from);
    if CRASH_ARMED.load(Ordering::Relaxed) && selected.as_deref() == Some(point) && directory.as_deref() == Some(base) {
        let pending = base.join("boundary-pending");
        fs::write(&pending, point)?;
        fs::rename(pending, base.join("boundary-reached"))?;
        loop {
            std::thread::park();
        }
    }
    Ok(())
}
