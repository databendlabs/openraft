use std::hint::black_box;

use criterion::Criterion;
use criterion::criterion_group;
use maplit::btreeset;
use openraft::bench_internals::QuorumSet;

fn bench_quorum_set_slice_ids_slice(c: &mut Criterion) {
    let m12345: &[usize] = &[1, 2, 3, 4, 5];
    let x = [1, 2, 3, 6, 7];
    c.bench_function("quorum_set_slice_ids_slice", |b| {
        b.iter(|| m12345.is_quorum(black_box(x.iter())))
    });
}

fn bench_quorum_set_btreeset_ids_slice(c: &mut Criterion) {
    let m12345678 = btreeset! {1,2,3,4,5,6,7,8};
    let x = [1, 2, 3, 6, 7];
    c.bench_function("quorum_set_btreeset_ids_slice", |b| {
        b.iter(|| m12345678.is_quorum(black_box(x.iter())))
    });
}

fn bench_quorum_set_vec_of_btreeset_ids_slice(c: &mut Criterion) {
    let m12345_678 = vec![btreeset! {1,2,3,4,5}, btreeset! {6,7,8}];
    let x = [1, 2, 3, 6, 7];
    c.bench_function("quorum_set_vec_of_btreeset_ids_slice", |b| {
        b.iter(|| m12345_678.is_quorum(black_box(x.iter())))
    });
}

fn bench_quorum_set_vec_of_btreeset_ids_btreeset(c: &mut Criterion) {
    let m12345_678 = vec![btreeset! {1,2,3,4,5}, btreeset! {6,7,8}];
    let x = btreeset! {1,2,3,6,7};
    c.bench_function("quorum_set_vec_of_btreeset_ids_btreeset", |b| {
        b.iter(|| m12345_678.is_quorum(black_box(x.iter())))
    });
}

criterion_group!(
    benches,
    bench_quorum_set_slice_ids_slice,
    bench_quorum_set_btreeset_ids_slice,
    bench_quorum_set_vec_of_btreeset_ids_slice,
    bench_quorum_set_vec_of_btreeset_ids_btreeset,
);
