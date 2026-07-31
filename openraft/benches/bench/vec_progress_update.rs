// Benchmarks for VecProgress::update with a joint quorum set.

use criterion::Criterion;
use criterion::criterion_group;
use openraft::bench_internals::BenchVecProgress;

fn bench_progress_update_01234_567(c: &mut Criterion) {
    let mut progress = BenchVecProgress::new_joint_01234_567();

    c.bench_function("progress_update_01234_567", |b| {
        b.iter(|| {
            progress.update_next();
        });
    });
}

criterion_group!(benches, bench_progress_update_01234_567);
