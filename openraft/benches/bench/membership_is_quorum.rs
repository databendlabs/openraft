// Benchmarks for Membership::is_quorum with various collection types.

use std::collections::BTreeSet;

use criterion::Criterion;
use criterion::black_box;
use criterion::criterion_group;
use openraft::bench_internals::membership_is_quorum_btreeset;
use openraft::bench_internals::membership_is_quorum_slice;
use openraft::bench_internals::new_membership;

fn bench_m12345_ids_slice(c: &mut Criterion) {
    let m = new_membership(vec![[1u64, 2, 3, 4, 5].into_iter().collect()]);
    let ids = [1u64, 2, 3, 6, 7];

    c.bench_function("m12345_ids_slice", |b| {
        b.iter(|| membership_is_quorum_slice(black_box(&ids), black_box(&m)));
    });
}

fn bench_m12345_ids_btreeset(c: &mut Criterion) {
    let m = new_membership(vec![[1u64, 2, 3, 4, 5].into_iter().collect()]);
    let ids: BTreeSet<u64> = [1, 2, 3, 6, 7].into_iter().collect();

    c.bench_function("m12345_ids_btreeset", |b| {
        b.iter(|| membership_is_quorum_btreeset(black_box(&ids), black_box(&m)));
    });
}

fn bench_m12345_678_ids_slice(c: &mut Criterion) {
    let m = new_membership(vec![[1u64, 2, 3, 4, 5].into_iter().collect()]);
    let ids = [1u64, 2, 3, 6, 7];

    c.bench_function("m12345_678_ids_slice", |b| {
        b.iter(|| membership_is_quorum_slice(black_box(&ids), black_box(&m)));
    });
}

fn bench_m12345_678_ids_btreeset(c: &mut Criterion) {
    let m = new_membership(vec![[1u64, 2, 3, 4, 5].into_iter().collect()]);
    let ids: BTreeSet<u64> = [1, 2, 3, 6, 7].into_iter().collect();

    c.bench_function("m12345_678_ids_btreeset", |b| {
        b.iter(|| membership_is_quorum_btreeset(black_box(&ids), black_box(&m)));
    });
}

criterion_group!(
    benches,
    bench_m12345_ids_slice,
    bench_m12345_ids_btreeset,
    bench_m12345_678_ids_slice,
    bench_m12345_678_ids_btreeset,
);
