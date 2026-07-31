// Benchmarks for LeaderHandler::leader_append_entries with varying batch sizes.

use criterion::Criterion;
use criterion::black_box;
use criterion::criterion_group;
use openraft::bench_internals::BenchEngine;
use openraft::bench_internals::BenchEntryPayload;

fn bench_leader_append_1_entry(c: &mut Criterion) {
    let mut eng = BenchEngine::new_leader();

    c.bench_function("leader_append_1_entry", |b| {
        b.iter(|| {
            eng.leader_append_entries(black_box([BenchEntryPayload::Blank]));
        });
    });
}

fn bench_leader_append_3_entries(c: &mut Criterion) {
    let mut eng = BenchEngine::new_leader();

    c.bench_function("leader_append_3_entries", |b| {
        b.iter(|| {
            eng.leader_append_entries(black_box([
                BenchEntryPayload::Blank,
                BenchEntryPayload::Blank,
                BenchEntryPayload::Blank,
            ]));
        });
    });
}

fn bench_leader_append_10_entries(c: &mut Criterion) {
    let mut eng = BenchEngine::new_leader();

    c.bench_function("leader_append_10_entries", |b| {
        b.iter(|| {
            eng.leader_append_entries(black_box([
                BenchEntryPayload::Blank,
                BenchEntryPayload::Blank,
                BenchEntryPayload::Blank,
                BenchEntryPayload::Blank,
                BenchEntryPayload::Blank,
                BenchEntryPayload::Blank,
                BenchEntryPayload::Blank,
                BenchEntryPayload::Blank,
                BenchEntryPayload::Blank,
                BenchEntryPayload::Blank,
            ]));
        });
    });
}

criterion_group!(
    benches,
    bench_leader_append_1_entry,
    bench_leader_append_3_entries,
    bench_leader_append_10_entries,
);
