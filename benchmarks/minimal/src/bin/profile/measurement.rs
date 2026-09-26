use std::time::Instant;

use serde_json::Value;

use super::Args;
use super::allocator;

pub(super) struct Measurement {
    started: Instant,
    allocations: usize,
    allocated_bytes: usize,
    live_bytes: usize,
}

impl Measurement {
    pub(super) fn start() -> Self {
        Self {
            started: Instant::now(),
            allocations: allocator::allocations(),
            allocated_bytes: allocator::allocated_bytes(),
            live_bytes: allocator::live_bytes(),
        }
    }

    pub(super) fn report(self, phase: &str, args: &Args, samples: Option<&mut [u64]>, operations: usize) {
        let elapsed = self.started.elapsed();
        let seconds = elapsed.as_secs_f64();
        let memory = self.memory();
        let p99_ns = percentile_99(samples);
        let operations_per_second = operations as f64 / seconds;
        let config = config(args);
        let report = serde_json::json!({
            "phase": phase,
            "config": config,
            "seconds": seconds,
            "operations": operations,
            "operations_per_second": operations_per_second,
            "request_p99_ns": p99_ns,
            "memory": memory,
        });
        println!("{report}");
    }

    fn memory(&self) -> Value {
        let allocations = allocator::allocations();
        let allocated_bytes = allocator::allocated_bytes();
        let live_bytes = allocator::live_bytes();
        let allocation_events = allocations - self.allocations;
        let requested_bytes = allocated_bytes - self.allocated_bytes;
        let live_delta = live_bytes as i128 - self.live_bytes as i128;
        serde_json::json!({
            "allocation_events": allocation_events,
            "requested_bytes": requested_bytes,
            "live_bytes_delta": live_delta,
        })
    }
}

fn percentile_99(samples: Option<&mut [u64]>) -> Option<u64> {
    let samples = samples?;
    if samples.is_empty() {
        return None;
    }
    samples.sort_unstable();
    let rank = (samples.len() * 99).div_ceil(100);
    let percentile = samples[rank - 1];
    Some(percentile)
}

fn config(args: &Args) -> Value {
    let runtime_stats = cfg!(feature = "runtime-stats");
    serde_json::json!({
        "clients": args.clients,
        "batch": args.batch,
        "members": args.members,
        "runtime_stats": runtime_stats,
    })
}

#[cfg(test)]
mod tests {
    use super::percentile_99;

    #[test]
    fn nearest_rank_percentile() {
        let mut samples: Vec<u64> = (1..=100).rev().collect();
        let percentile = percentile_99(Some(&mut samples));
        assert_eq!(percentile, Some(99));

        let mut samples = [300, 100, 200];
        let percentile = percentile_99(Some(&mut samples));
        assert_eq!(percentile, Some(300));

        let mut samples = [];
        let percentile = percentile_99(Some(&mut samples));
        assert_eq!(percentile, None);
    }
}
