// Benchmarks for leader_append_entries with varying batch sizes.
//
// Run with:
//   cargo bench --features bench -p openraft -- leader_append

use std::hint::black_box;
use std::time::Duration;
use std::time::Instant;

use criterion::Criterion;
use criterion::criterion_group;
use openraft::bench_internals::BenchEngine;
use openraft::bench_internals::UTConfig;
use openraft::type_config::alias::EntryPayloadOf;

type Payload = EntryPayloadOf<UTConfig>;

/// How many appends to time before draining the engine's command buffer.
///
/// Appending queues a command, so the buffer has to be drained or it grows without bound.
/// Draining is bookkeeping rather than work under test, so it happens between timed batches. The
/// batch also amortizes the two `Instant` reads over 64 appends.
const DRAIN_INTERVAL: u64 = 64;

fn bench_append<const N: usize>(c: &mut Criterion, name: &str) {
    let mut eng = BenchEngine::new_leader();

    c.bench_function(name, |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            let mut remaining = iters;

            while remaining > 0 {
                let batch = remaining.min(DRAIN_INTERVAL);

                let start = Instant::now();
                for _ in 0..batch {
                    let payloads: [Payload; N] = std::array::from_fn(|_| Payload::Blank);
                    eng.append(black_box(payloads));
                }
                elapsed += start.elapsed();

                eng.clear_commands();
                remaining -= batch;
            }

            elapsed
        })
    });
}

fn bench_leader_append_1_entry(c: &mut Criterion) {
    bench_append::<1>(c, "leader_append_1_entry");
}

fn bench_leader_append_3_entries(c: &mut Criterion) {
    bench_append::<3>(c, "leader_append_3_entries");
}

fn bench_leader_append_10_entries(c: &mut Criterion) {
    bench_append::<10>(c, "leader_append_10_entries");
}

criterion_group!(
    benches,
    bench_leader_append_1_entry,
    bench_leader_append_3_entries,
    bench_leader_append_10_entries,
);
