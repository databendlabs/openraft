// Criterion benchmark entry point.
//
// Run all:   cargo bench --features bench -p openraft
// Filter:    cargo bench --features bench -p openraft -- leader_append

mod bench;

use criterion::criterion_main;

criterion_main!(
    bench::leader_append_entries::benches,
    bench::vec_progress_update::benches,
    bench::quorum_is_quorum::benches,
    bench::membership_is_quorum::benches,
);
