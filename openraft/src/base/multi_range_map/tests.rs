use super::MultiRangeMap;

type M = MultiRangeMap<u64, u64, 3>;

fn collect_segments(m: &M, begin: u64) -> Vec<(std::ops::Range<u64>, [u64; 3])> {
    m.segments(begin).map(|s| (s.range, s.values)).collect()
}

#[test]
fn test_segments_empty() {
    let m = M::new(10);
    assert_eq!(collect_segments(&m, 0), vec![]);
}

#[test]
fn test_segments_single_aligned() {
    let mut m = M::new(10);
    m.get_mut(0).record(10, 1);
    m.get_mut(1).record(10, 2);
    m.get_mut(2).record(10, 3);

    assert_eq!(collect_segments(&m, 0), vec![(0..10, [1, 2, 3])]);
}

#[test]
fn test_segments_two_aligned() {
    let mut m = M::new(10);
    for i in 0..3 {
        m.get_mut(i).record(10, (i as u64 + 1) * 10);
        m.get_mut(i).record(20, (i as u64 + 1) * 100);
    }

    assert_eq!(collect_segments(&m, 0), vec![
        (0..10, [10, 20, 30]),
        (10..20, [100, 200, 300]),
    ]);
}

#[test]
fn test_segments_misaligned() {
    let mut m = M::new(10);

    // Map 0: boundaries at 10, 20
    m.get_mut(0).record(10, 1);
    m.get_mut(0).record(20, 2);

    // Map 1: boundaries at 15, 20
    m.get_mut(1).record(15, 10);
    m.get_mut(1).record(20, 20);

    // Map 2: boundaries at 10, 20
    m.get_mut(2).record(10, 100);
    m.get_mut(2).record(20, 200);

    // Merged boundaries: 10, 15, 20
    // Segment [0,10): map0=1, map1=10, map2=100
    // Segment [10,15): map0=2, map1=10, map2=200
    // Segment [15,20): map0=2, map1=20, map2=200
    assert_eq!(collect_segments(&m, 0), vec![
        (0..10, [1, 10, 100]),
        (10..15, [2, 10, 200]),
        (15..20, [2, 20, 200]),
    ]);
}

#[test]
fn test_segments_partial_data() {
    // One map has fewer entries — segments stop at min_end
    let mut m = M::new(10);
    m.get_mut(0).record(10, 1);
    m.get_mut(0).record(20, 2);
    m.get_mut(1).record(10, 10);
    // map 1 stops at 10, map 2 goes to 20
    m.get_mut(2).record(10, 100);
    m.get_mut(2).record(20, 200);

    // min_end = 10, so only one segment
    assert_eq!(collect_segments(&m, 0), vec![(0..10, [1, 10, 100])]);
}

#[test]
fn test_segments_after_eviction() {
    let mut m = M::new(2);
    let mut begin = 0u64;

    for i in 0..3 {
        m.get_mut(i).record(10, (i as u64 + 1) * 10);
        m.get_mut(i).record(20, (i as u64 + 1) * 100);
    }

    // Record a third entry, evicting the first from each map
    for i in 0..3 {
        if let Some(evicted) = m.get_mut(i).record(30, (i as u64 + 1) * 1000) {
            begin = begin.max(evicted);
        }
    }

    // begin advanced to 10, remaining entries: (20, ..) and (30, ..)
    assert_eq!(begin, 10);
    assert_eq!(collect_segments(&m, begin), vec![
        (10..20, [100, 200, 300]),
        (20..30, [1000, 2000, 3000]),
    ]);
}

#[test]
fn test_segments_one_map_empty() {
    // If any map is empty, min_end falls back to begin → no segments
    let mut m = M::new(10);
    m.get_mut(0).record(10, 1);
    m.get_mut(1).record(10, 2);
    // map 2 is empty

    assert_eq!(collect_segments(&m, 0), vec![]);
}
