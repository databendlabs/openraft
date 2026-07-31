use std::hint::black_box;

use criterion::Criterion;
use criterion::criterion_group;
use openraft::bench_internals::BenchVecProgress;

fn bench_progress_update_01234_567(c: &mut Criterion) {
    let mut progress = BenchVecProgress::new_joint_01234_567();

    let mut id = 0u64;
    let mut values = [0, 1, 2, 3, 4, 5, 6, 7];
    c.bench_function("progress_update_01234_567", |b| {
        b.iter(|| {
            id = (id + 1) & 7;
            values[id as usize] += 1;
            let v = values[id as usize];

            progress.update(black_box(id), black_box(v));
        })
    });

    // It shows that is_quorum() is called at a rate of about 1/4 of update()
    // `Stat { update_count: 42997501, move_count: 10749381, is_quorum_count: 10749399 }`
}

criterion_group!(benches, bench_progress_update_01234_567);
