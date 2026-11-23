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
fn test_record_stage_backfills_earlier_stages() {
    // Simulate a follower: only stages from Submitted onward are recorded
    // locally. Earlier stages (Proposed, Received) must be back-filled so
    // the segment intersection still produces a segment.
    let base = C::now();
    let mut latency = LS::new(10, 0);

    let t_submitted = instant_at(base, 2);
    let t_persisted = instant_at(base, 5);
    let t_committed = instant_at(base, 7);
    let t_applied = instant_at(base, 10);

    latency.submitted(11, t_submitted);
    latency.persisted(11, t_persisted);
    latency.committed(11, t_committed);
    latency.applied(11, t_applied);

    let segments: Vec<_> = latency.segments().collect();
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].range, 0..11);

    // Proposed and Received are back-filled with the first recorded value
    // (Submitted's timestamp), so the step duration from Proposed to
    // Received and from Received to Submitted are both zero.
    assert_eq!(segments[0].values[Stage::Proposed.index()], t_submitted);
    assert_eq!(segments[0].values[Stage::Received.index()], t_submitted);
    assert_eq!(segments[0].values[Stage::Submitted.index()], t_submitted);
    assert_eq!(segments[0].values[Stage::Persisted.index()], t_persisted);
    assert_eq!(segments[0].values[Stage::Committed.index()], t_committed);
    assert_eq!(segments[0].values[Stage::Applied.index()], t_applied);
}

#[test]
fn test_record_stage_backfill_skips_stages_already_covered() {
    // If earlier stages already cover `right`, back-fill must not
    // overwrite them (the leader path: all stages recorded in order
    // at the same right boundary).
    let base = C::now();
    let mut latency = LS::new(10, 0);

    let t_proposed = instant_at(base, 1);
    let t_received = instant_at(base, 2);
    let t_submitted = instant_at(base, 3);

    latency.proposed(11, t_proposed);
    latency.received(11, t_received);
    latency.submitted(11, t_submitted);

    // Submitted's back-fill should not touch Proposed or Received
    // because both already end at 11.
    latency.persisted(11, instant_at(base, 4));
    latency.committed(11, instant_at(base, 5));
    latency.applied(11, instant_at(base, 6));

    let segments: Vec<_> = latency.segments().collect();
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].values[Stage::Proposed.index()], t_proposed);
    assert_eq!(segments[0].values[Stage::Received.index()], t_received);
    assert_eq!(segments[0].values[Stage::Submitted.index()], t_submitted);
}

#[test]
fn test_record_stage_backfill_extends_lagging_stage() {
    // A later stage advancing to a new right boundary must back-fill any
    // earlier stage that is still behind, even if that earlier stage has
    // some data.
    let base = C::now();
    let mut latency = LS::new(10, 0);

    let t1 = instant_at(base, 1);
    let t2 = instant_at(base, 10);

    // First batch: all stages at right=10.
    record_all(&mut latency, 10, [t1; 6]);

    // Second batch: only Submitted onward recorded at right=20. Proposed
    // and Received end at 10 and must be back-filled to 20.
    latency.submitted(20, t2);
    latency.persisted(20, t2);
    latency.committed(20, t2);
    latency.applied(20, t2);

    let segments: Vec<_> = latency.segments().collect();
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].range, 0..10);
    assert_eq!(segments[1].range, 10..20);
    assert_eq!(segments[1].values[Stage::Proposed.index()], t2);
    assert_eq!(segments[1].values[Stage::Received.index()], t2);
    assert_eq!(segments[1].values[Stage::Submitted.index()], t2);
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

/// The segments as `(range, values)`.
fn segments_of(latency: &LS) -> Vec<(std::ops::Range<u64>, [InstantOf<C>; 6])> {
    latency.segments().map(|seg| (seg.range, seg.values)).collect()
}

/// Truncating at a batch boundary removes the batches from it on and keeps the ones before it.
#[test]
fn test_truncate_at_a_batch_boundary() {
    let base = C::now();
    let mut latency = LS::new(10, 0);
    let t1 = [0, 1, 2, 3, 4, 5].map(|i| instant_at(base, i));
    let t2 = [10, 11, 12, 13, 14, 15].map(|i| instant_at(base, i));
    let t3 = [20, 21, 22, 23, 24, 25].map(|i| instant_at(base, i));
    record_all(&mut latency, 2, t1);
    record_all(&mut latency, 4, t2);
    record_all(&mut latency, 6, t3);

    latency.truncate(4);
    assert_eq!(vec![(0..2, t1), (2..4, t2)], segments_of(&latency));

    let t4 = [30, 31, 32, 33, 34, 35].map(|i| instant_at(base, i));
    record_all(&mut latency, 6, t4);
    assert_eq!(vec![(0..2, t1), (2..4, t2), (4..6, t4)], segments_of(&latency));
}

/// Truncating inside a batch keeps the batch's stages for the logs before the boundary.
#[test]
fn test_truncate_inside_a_batch() {
    let base = C::now();
    let mut latency = LS::new(10, 0);
    let t1 = [0, 1, 2, 3, 4, 5].map(|i| instant_at(base, i));
    record_all(&mut latency, 6, t1);

    latency.truncate(4);
    assert_eq!(vec![(0..4, t1)], segments_of(&latency));

    let t2 = [10, 11, 12, 13, 14, 15].map(|i| instant_at(base, i));
    record_all(&mut latency, 5, t2);
    assert_eq!(vec![(0..4, t1), (4..5, t2)], segments_of(&latency));
}

/// A follower's replacement of a discarded log is backfilled from its own submission, whether it
/// ends at the discarded batch's right boundary or before it.
#[test]
fn test_truncated_log_is_backfilled_from_its_replacement() {
    for replacement_end in [3, 2] {
        let base = C::now();
        let mut latency = LS::new(10, 0);
        latency.submitted(3, instant_at(base, 0));
        latency.persisted(3, instant_at(base, 1));

        latency.truncate(0);
        assert_eq!(
            Vec::<(std::ops::Range<u64>, [InstantOf<C>; 6])>::new(),
            segments_of(&latency)
        );

        let t = [10, 11, 12, 13].map(|i| instant_at(base, i));
        latency.submitted(replacement_end, t[0]);
        latency.persisted(replacement_end, t[1]);
        latency.committed(replacement_end, t[2]);
        latency.applied(replacement_end, t[3]);

        let expected = [t[0], t[0], t[0], t[1], t[2], t[3]];
        assert_eq!(vec![(0..replacement_end, expected)], segments_of(&latency));
    }
}

/// A truncation before the retained history discards the history and restarts the observation at
/// the boundary, so that the replacement is observed although eviction moved the start past it.
#[test]
fn test_truncate_before_the_retained_start() {
    let base = C::now();
    let mut latency = LS::new(2, 0);
    let mut recorded = vec![];
    for right in 1..=3 {
        let t = [0, 1, 2, 3, 4, 5].map(|i| instant_at(base, right * 10 + i));
        record_all(&mut latency, right, t);
        recorded.push((right - 1..right, t));
    }
    assert_eq!(recorded[1..], segments_of(&latency), "eviction moved the start to 1");

    latency.truncate(0);
    assert_eq!(
        Vec::<(std::ops::Range<u64>, [InstantOf<C>; 6])>::new(),
        segments_of(&latency)
    );

    let t = [100, 101, 102, 103].map(|i| instant_at(base, i));
    latency.submitted(1, t[0]);
    latency.persisted(1, t[1]);
    latency.committed(1, t[2]);
    latency.applied(1, t[3]);

    let expected = [t[0], t[0], t[0], t[1], t[2], t[3]];
    assert_eq!(vec![(0..1, expected)], segments_of(&latency));
    // Without the feature the histograms record nothing.
    #[cfg(feature = "runtime-stats")]
    assert_eq!(1, latency.compute_histograms().proposed_to_applied.total());
}

/// A truncation after every recorded batch changes nothing.
#[test]
fn test_truncate_after_the_recorded_logs() {
    let base = C::now();
    let mut latency = LS::new(10, 0);
    let t1 = [0, 1, 2, 3, 4, 5].map(|i| instant_at(base, i));
    record_all(&mut latency, 3, t1);

    let before = latency.clone();
    latency.truncate(3);
    assert_eq!(before, latency);
    latency.truncate(5);
    assert_eq!(before, latency);
}

/// Excluding the logs an installed snapshot covers keeps the observed prefix before them and the
/// suffix queued after them, and drops the segment in between.
#[test]
fn test_exclude_keeps_the_observed_prefix_and_the_queued_suffix() {
    let base = C::now();
    let mut latency = LS::new(10, 0);
    let t1 = [0, 1, 2, 3, 4, 5].map(|i| instant_at(base, i));
    record_all(&mut latency, 5, t1);

    // The suffix after the snapshot is submitted and persisted before the snapshot result arrives.
    let queued = [10, 11].map(|i| instant_at(base, i));
    latency.submitted(24, queued[0]);
    latency.persisted(24, queued[1]);

    latency.exclude(5..21);

    let done = [20, 21].map(|i| instant_at(base, i));
    latency.committed(24, done[0]);
    latency.applied(24, done[1]);

    let suffix = [queued[0], queued[0], queued[0], queued[1], done[0], done[1]];
    assert_eq!(vec![(0..5, t1), (21..24, suffix)], segments_of(&latency));
    // Without the feature the histograms record nothing.
    #[cfg(feature = "runtime-stats")]
    assert_eq!(5 + 3, latency.compute_histograms().proposed_to_applied.total());
}

/// A node that installs a snapshot before it observes any log starts the observation after the
/// snapshot: an excluded range at the observation start moves the start to its end.
#[test]
fn test_exclude_at_the_start_moves_the_start() {
    let base = C::now();
    let mut latency = LS::new(10, 0);
    latency.exclude(0..7);

    let t = [0, 1, 2, 3, 4, 5].map(|i| instant_at(base, i));
    record_all(&mut latency, 8, t);

    assert_eq!(vec![(7..8, t)], segments_of(&latency));
    // Without the feature the histograms record nothing.
    #[cfg(feature = "runtime-stats")]
    assert_eq!(1, latency.compute_histograms().proposed_to_applied.total());
}

/// Overlapping exclusions merge, and when they outnumber the stage capacity, the history before the
/// oldest one is forgotten.
#[test]
fn test_exclude_merges_and_stays_bounded() {
    let base = C::now();
    let mut latency = LS::new(2, 0);
    let t1 = [0, 1, 2, 3, 4, 5].map(|i| instant_at(base, i));
    record_all(&mut latency, 1, t1);

    latency.exclude(2..4);
    latency.exclude(3..6);
    latency.exclude(7..8);

    let t2 = [10, 11, 12, 13, 14, 15].map(|i| instant_at(base, i));
    record_all(&mut latency, 9, t2);
    assert_eq!(
        vec![(0..1, t1), (1..2, t2), (6..7, t2), (8..9, t2)],
        segments_of(&latency),
        "2..4 and 3..6 merge"
    );

    latency.exclude(10..11);
    assert_eq!(
        vec![(6..7, t2), (8..9, t2)],
        segments_of(&latency),
        "a third range forgets the history before 2..6"
    );
}
