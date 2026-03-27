use std::time::Duration;

use crate::core::runtime_stats::log_stage::LogStages;
use crate::core::stage::Stage;
use crate::engine::testing::UTConfig;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::InstantOf;

type C = UTConfig<()>;
type LS = LogStages<InstantOf<C>>;

fn instant_at(base: InstantOf<C>, millis: u64) -> InstantOf<C> {
    base + Duration::from_millis(millis)
}

fn record_all(latency: &mut LS, right_boundary: u64, times: [InstantOf<C>; 6]) {
    latency.proposed(right_boundary, times[0]);
    latency.received(right_boundary, times[1]);
    latency.submitted(right_boundary, times[2]);
    latency.persisted(right_boundary, times[3]);
    latency.committed(right_boundary, times[4]);
    latency.applied(right_boundary, times[5]);
}

#[test]
fn test_segment_iter_empty() {
    let latency = LS::new(10, 0);
    let segments: Vec<_> = latency.segments().collect();
    assert!(segments.is_empty());
}

#[test]
fn test_segment_iter_single_batch() {
    let base = C::now();
    let mut latency = LS::new(10, 0);

    let t = [0, 1, 2, 3, 4, 5].map(|i| instant_at(base, i));
    latency.proposed(11, t[0]);
    latency.received(11, t[1]);
    latency.submitted(11, t[2]);
    latency.persisted(11, t[3]);
    latency.committed(11, t[4]);
    latency.applied(11, t[5]);

    let segments: Vec<_> = latency.segments().collect();
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].range, 0..11);
    assert_eq!(segments[0].values[Stage::Proposed.index()], t[0]);
    assert_eq!(segments[0].values[Stage::Applied.index()], t[5]);
}

#[test]
fn test_segment_iter_two_batches_aligned() {
    let base = C::now();
    let mut latency = LS::new(10, 0);

    let t1 = [0, 1, 2, 3, 4, 5].map(|i| instant_at(base, i));
    let t2 = [10, 11, 12, 13, 14, 15].map(|i| instant_at(base, i));

    record_all(&mut latency, 10, t1);
    record_all(&mut latency, 20, t2);

    let segments: Vec<_> = latency.segments().collect();
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].range, 0..10);
    assert_eq!(segments[0].values[Stage::Proposed.index()], t1[0]);
    assert_eq!(segments[0].values[Stage::Applied.index()], t1[5]);
    assert_eq!(segments[1].range, 10..20);
    assert_eq!(segments[1].values[Stage::Proposed.index()], t2[0]);
    assert_eq!(segments[1].values[Stage::Applied.index()], t2[5]);
}

#[test]
fn test_segment_iter_misaligned_boundaries() {
    let base = C::now();
    let mut latency = LS::new(10, 0);

    let t = |ms: u64| instant_at(base, ms);

    latency.proposed(10, t(0));
    latency.proposed(20, t(10));

    latency.received(15, t(1));
    latency.received(20, t(11));

    latency.submitted(10, t(2));
    latency.submitted(20, t(12));

    latency.persisted(10, t(3));
    latency.persisted(20, t(13));

    latency.committed(10, t(4));
    latency.committed(20, t(14));

    latency.applied(10, t(5));
    latency.applied(20, t(15));

    let segments: Vec<_> = latency.segments().collect();

    assert_eq!(segments.len(), 3);
    assert_eq!(segments[0].range, 0..10);
    assert_eq!(segments[1].range, 10..15);
    assert_eq!(segments[2].range, 15..20);

    assert_eq!(segments[1].values[Stage::Proposed.index()], t(10));
    assert_eq!(segments[1].values[Stage::Received.index()], t(1));

    assert_eq!(segments[2].values[Stage::Received.index()], t(11));
}

#[test]
fn test_segment_iter_different_begin() {
    let base = C::now();
    let t = |ms: u64| instant_at(base, ms);

    let mut latency = LS::new(2, 0);

    let t1 = [t(1); 6];
    let t2 = [t(2); 6];
    let t3 = [t(3); 6];
    record_all(&mut latency, 10, t1);
    record_all(&mut latency, 20, t2);
    record_all(&mut latency, 30, t3);

    let segments: Vec<_> = latency.segments().collect();
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].range, 10..20);
    assert_eq!(segments[0].values[Stage::Proposed.index()], t(2));
    assert_eq!(segments[1].range, 20..30);
    assert_eq!(segments[1].values[Stage::Proposed.index()], t(3));
}

#[test]
fn test_display_empty() {
    let latency = LS::new(10, 0);
    assert_eq!("", format!("{}", latency));
}

#[test]
fn test_display_one_line_per_segment() {
    let base = C::now();
    let mut latency = LS::new(10, 0);

    record_all(&mut latency, 10, [0, 1, 2, 3, 4, 5].map(|i| instant_at(base, i)));
    record_all(&mut latency, 20, [10, 11, 12, 13, 14, 15].map(|i| instant_at(base, i)));

    let s = format!("{}", latency);
    let lines: Vec<_> = s.lines().collect();

    assert_eq!(2, lines.len());
    assert!(lines[0].starts_with("[0,10): "));
    assert!(lines[1].starts_with("[10,20): "));
    assert!(lines[0].contains(" +0.00ns; "));
    assert!(lines[1].contains(" +10.00ms; "));

    for line in lines {
        assert!(line.contains("proposed +0.00ns (0.00ns)"));
        assert!(line.contains("received +"));
        assert!(line.contains("submitted +"));
        assert!(line.contains("persisted +"));
        assert!(line.contains("committed +"));
        assert!(line.contains("applied +"));
    }
}

#[test]
fn test_display_cumulative_duration_follows_stage_order() {
    let base = C::now();
    let mut latency = LS::new(10, 0);

    latency.proposed(1, instant_at(base, 10));
    latency.received(1, instant_at(base, 11));
    latency.submitted(1, instant_at(base, 10));
    latency.persisted(1, instant_at(base, 12));
    latency.committed(1, instant_at(base, 13));
    latency.applied(1, instant_at(base, 15));

    let s = format!("{}", latency);

    assert!(s.contains("proposed +0.00ns (0.00ns)"));
    assert!(s.contains("received +1.00ms (1.00ms)"));
    assert!(s.contains("submitted +0.00ns (1.00ms)"));
    assert!(s.contains("persisted +2.00ms (3.00ms)"));
    assert!(s.contains("committed +1.00ms (4.00ms)"));
    assert!(s.contains("applied +2.00ms (6.00ms)"));
}

#[test]
fn test_display_duration_uses_at_most_two_fraction_digits() {
    let base = C::now();
    let mut latency = LS::new(10, 0);

    latency.proposed(1, base);
    latency.received(1, base + Duration::from_nanos(223_584));
    latency.submitted(1, base + Duration::from_nanos(1_223_584));
    latency.persisted(1, base + Duration::from_nanos(3_568_584));
    latency.committed(1, base + Duration::from_nanos(3_786_209));
    latency.applied(1, base + Duration::from_nanos(4_477_209));

    let s = format!("{}", latency);

    assert!(s.contains("223.58"));
    assert!(s.contains("1.22ms"));
    assert!(!s.contains("223.584"));
}
