// Benchmarks for QuorumSet::is_quorum with various collection types.

use std::collections::BTreeSet;

use criterion::Criterion;
use criterion::black_box;
use criterion::criterion_group;
use openraft::bench_internals::quorum_btreeset_is_quorum_slice;
use openraft::bench_internals::quorum_joint_is_quorum_btreeset;
use openraft::bench_internals::quorum_joint_is_quorum_slice;
use openraft::bench_internals::quorum_slice_is_quorum_slice;

fn bench_quorum_set_slice_ids_slice(c: &mut Criterion) {
    let quorum_set: Vec<usize> = vec![1, 2, 3, 4, 5];
    let ids = [1, 2, 3, 6, 7];

    c.bench_function("quorum_set_slice_ids_slice", |b| {
        b.iter(|| quorum_slice_is_quorum_slice(black_box(&ids), black_box(&quorum_set)));
    });
}

fn bench_quorum_set_btreeset_ids_slice(c: &mut Criterion) {
    let quorum_set: BTreeSet<usize> = [1, 2, 3, 4, 5, 6, 7, 8].into_iter().collect();
    let ids = [1, 2, 3, 6, 7];

    c.bench_function("quorum_set_btreeset_ids_slice", |b| {
        b.iter(|| quorum_btreeset_is_quorum_slice(black_box(&ids), black_box(&quorum_set)));
    });
}

fn bench_quorum_set_vec_of_btreeset_ids_slice(c: &mut Criterion) {
    let quorum_set: Vec<BTreeSet<usize>> = vec![[1, 2, 3, 4, 5].into_iter().collect(), [6, 7, 8].into_iter().collect()];
    let ids = [1, 2, 3, 6, 7];

    c.bench_function("quorum_set_vec_of_btreeset_ids_slice", |b| {
        b.iter(|| quorum_joint_is_quorum_slice(black_box(&ids), black_box(&quorum_set)));
    });
}

fn bench_quorum_set_vec_of_btreeset_ids_btreeset(c: &mut Criterion) {
    let quorum_set: Vec<BTreeSet<usize>> = vec![[1, 2, 3, 4, 5].into_iter().collect(), [6, 7, 8].into_iter().collect()];
    let ids: BTreeSet<usize> = [1, 2, 3, 6, 7].into_iter().collect();

    c.bench_function("quorum_set_vec_of_btreeset_ids_btreeset", |b| {
        b.iter(|| quorum_joint_is_quorum_btreeset(black_box(&ids), black_box(&quorum_set)));
    });
}

criterion_group!(
    benches,
    bench_quorum_set_slice_ids_slice,
    bench_quorum_set_btreeset_ids_slice,
    bench_quorum_set_vec_of_btreeset_ids_slice,
    bench_quorum_set_vec_of_btreeset_ids_btreeset,
);
