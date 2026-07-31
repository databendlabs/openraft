use std::hint::black_box;

use criterion::Criterion;
use criterion::criterion_group;
use maplit::btreeset;
use openraft::Membership;
use openraft::bench_internals::QuorumSet;
use openraft::bench_internals::UTConfig;
use openraft::type_config::alias::StoredMembershipOf;

fn bench_m12345_ids_slice(c: &mut Criterion) {
    let m = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3,4,5}], None);
    let m = StoredMembershipOf::<UTConfig>::new(None, m);
    let x = [1, 2, 3, 6, 7];

    c.bench_function("m12345_ids_slice", |b| b.iter(|| m.is_quorum(black_box(x.iter()))));
}

fn bench_m12345_ids_btreeset(c: &mut Criterion) {
    let m = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3,4,5}], None);
    let m = StoredMembershipOf::<UTConfig>::new(None, m);
    let x = btreeset! {1, 2, 3, 6, 7};

    c.bench_function("m12345_ids_btreeset", |b| b.iter(|| m.is_quorum(black_box(x.iter()))));
}

fn bench_m12345_678_ids_slice(c: &mut Criterion) {
    let m = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3,4,5}], None);
    let m = StoredMembershipOf::<UTConfig>::new(None, m);
    let x = [1, 2, 3, 6, 7];

    c.bench_function("m12345_678_ids_slice", |b| b.iter(|| m.is_quorum(black_box(x.iter()))));
}

fn bench_m12345_678_ids_btreeset(c: &mut Criterion) {
    let m = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3,4,5}], None);
    let m = StoredMembershipOf::<UTConfig>::new(None, m);
    let x = btreeset! {1, 2, 3, 6, 7};

    c.bench_function("m12345_678_ids_btreeset", |b| {
        b.iter(|| m.is_quorum(black_box(x.iter())))
    });
}

criterion_group!(
    benches,
    bench_m12345_ids_slice,
    bench_m12345_ids_btreeset,
    bench_m12345_678_ids_slice,
    bench_m12345_678_ids_btreeset,
);
