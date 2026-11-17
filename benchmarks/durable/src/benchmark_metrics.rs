//! Optional HTTP body counters for the separate-process benchmark.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::OnceLock;

static METRICS: OnceLock<Mutex<BTreeMap<String, [u64; 3]>>> = OnceLock::new();

/// Enable per-route request and HTTP body byte counters for one example process.
#[openraft::macros::since(version = "0.10.1")]
pub fn enable() {
    METRICS.get_or_init(|| Mutex::new(BTreeMap::new()));
}

pub(crate) fn request(path: String, bytes: usize) {
    let metrics = METRICS.get().expect("benchmark metrics enabled");
    let mut metrics = metrics.lock().unwrap();
    let counters = metrics.entry(path).or_default();
    counters[0] += 1;
    counters[1] += bytes as u64;
}

pub(crate) fn response(path: &str, bytes: usize) {
    let metrics = METRICS.get().expect("benchmark metrics enabled");
    let mut metrics = metrics.lock().unwrap();
    let counters = metrics.get_mut(path).expect("request counted before response");
    counters[2] += bytes as u64;
}

/// Return counters, including the leader's follower-authorization endpoint.
#[openraft::macros::since(version = "0.10.1")]
pub fn report() -> BTreeMap<String, [u64; 3]> {
    let Some(metrics) = METRICS.get() else {
        return BTreeMap::new();
    };
    metrics.lock().unwrap().clone()
}
