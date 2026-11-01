use super::*;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;

/// Gives each queued request a distinct timestamp without relying on wall-clock delays.
fn spread_sending_times(q: &mut InflightAppendQueue<UTConfig>) -> Vec<InstantOf<UTConfig>> {
    let queue = &mut q.queue;
    let base = queue[0].sending_time;
    let mut times = Vec::new();
    for (index, request) in queue.iter_mut().enumerate() {
        let offset = std::time::Duration::from_secs(index as u64);
        request.sending_time = base + offset;
        times.push(request.sending_time);
    }
    times
}

#[test]
fn test_new() {
    let mut q = InflightAppendQueue::<UTConfig>::new();
    let actual = q.drain_acked(&None);
    assert_eq!(actual, None);
}

#[test]
fn test_drain_conflicted() {
    let mut q = InflightAppendQueue::<UTConfig>::new();
    q.push(Some(log_id(1, 1, 0)), Some(log_id(1, 1, 10)));
    q.push(Some(log_id(1, 1, 10)), Some(log_id(1, 1, 10)));
    q.push(Some(log_id(1, 1, 10)), Some(log_id(1, 1, 30)));

    let times = spread_sending_times(&mut q);
    // An entry-less segment derived from the first request can also reject prev=10.
    let conflict = log_id(1, 1, 10);
    let actual = q.drain_conflicted(&conflict);
    assert_eq!(actual, Some(times[0]));

    // Conflict(25) falls inside C's range.
    let conflict = log_id(1, 1, 25);
    let actual = q.drain_conflicted(&conflict);
    assert_eq!(actual, Some(times[2]));

    {
        let queue = &q.queue;
        let empty = queue.is_empty();
        assert!(empty);
    }

    let conflict = log_id(1, 1, 30);
    let actual = q.drain_conflicted(&conflict);
    assert_eq!(actual, None);
}

#[test]
fn test_push_preserves_requests() {
    let mut q = InflightAppendQueue::<UTConfig>::new();
    let first = Some(log_id(1, 1, 5));
    let second = Some(log_id(1, 1, 10));
    q.push(None, first);
    q.push(first, first);
    q.push(first, second);
    let acknowledged = q.drain_acked(&None);
    assert_eq!(acknowledged, None);
    let queue = &q.queue;
    let requests = queue.iter();
    let log_ids = requests.map(|request| request.last_log_id);
    let actual = log_ids.collect::<Vec<_>>();
    let expected = vec![first, first, second];
    assert_eq!(actual, expected);
}

#[test]
fn test_drain_conflicted_after_ack_uses_retained_time() {
    let mut q = InflightAppendQueue::<UTConfig>::new();
    let boundary = Some(log_id(1, 1, 10));
    q.push(None, boundary);
    q.push(boundary, Some(log_id(1, 1, 20)));
    let times = spread_sending_times(&mut q);
    let acknowledged = q.drain_acked(&boundary);
    assert_eq!(acknowledged, Some(times[0]));
    let conflict = log_id(1, 1, 5);
    let conflicted = q.drain_conflicted(&conflict);
    assert_eq!(conflicted, None);
    let conflict = log_id(1, 1, 10);
    let conflicted = q.drain_conflicted(&conflict);
    assert_eq!(conflicted, Some(times[0]));
}

#[test]
fn test_drain_acked_progress_and_stale_responses() {
    let mut q = InflightAppendQueue::<UTConfig>::new();
    q.push(Some(log_id(1, 1, 0)), Some(log_id(1, 1, 10)));
    q.push(Some(log_id(1, 1, 10)), Some(log_id(1, 1, 10)));
    q.push(Some(log_id(1, 1, 10)), Some(log_id(1, 1, 20)));
    let times = spread_sending_times(&mut q);
    let cases = [
        ("partial first request", 5, Some(times[0]), false),
        ("first boundary", 10, Some(times[0]), false),
        ("repeated boundary", 10, Some(times[0]), false),
        ("partial next request", 15, Some(times[2]), false),
        // Decreasing progress exercises defensive handling of stale responses.
        ("stale retained boundary", 10, Some(times[0]), false),
        ("all requests acknowledged", 20, Some(times[2]), true),
        ("behind retained boundary", 10, None, true),
        ("retained boundary after drain", 20, Some(times[2]), true),
    ];
    for (case, index, expected, expected_empty) in cases {
        let matching = Some(log_id(1, 1, index));
        let actual = q.drain_acked(&matching);
        assert_eq!(actual, expected, "{case}");
        let empty = q.is_empty();
        assert_eq!(empty, expected_empty, "{case}");
    }
}

#[test]
fn test_drain_acked_cumulative_partial() {
    let mut q = InflightAppendQueue::<UTConfig>::new();
    q.push(Some(log_id(1, 1, 0)), Some(log_id(1, 1, 10)));
    q.push(Some(log_id(1, 1, 10)), Some(log_id(1, 1, 20)));
    let times = spread_sending_times(&mut q);

    let matching = Some(log_id(1, 1, 15));
    let first = q.drain_acked(&matching);
    let matching = Some(log_id(1, 1, 20));
    let second = q.drain_acked(&matching);
    let actual = [first, second];
    let expected = [Some(times[1]), Some(times[1])];
    assert_eq!(actual, expected);
    let empty = q.is_empty();
    assert!(empty);
}

#[test]
fn test_drain_acked_same_last_log_id() {
    let mut q = InflightAppendQueue::<UTConfig>::new();
    q.push(None, Some(log_id(1, 1, 10)));
    q.push(Some(log_id(1, 1, 10)), Some(log_id(1, 1, 10)));
    q.push(Some(log_id(1, 1, 10)), Some(log_id(1, 1, 20)));
    let times = spread_sending_times(&mut q);

    let matching = Some(log_id(1, 1, 10));
    let first = q.drain_acked(&matching);
    let second = q.drain_acked(&matching);
    let matching = Some(log_id(1, 1, 20));
    let third = q.drain_acked(&matching);
    let actual = [first, second, third];
    let expected = [Some(times[0]), Some(times[0]), Some(times[2])];
    assert_eq!(actual, expected);
    let empty = q.is_empty();
    assert!(empty);
}

#[test]
fn test_drain_acked_same_last_log_id_across_responses() {
    let mut q = InflightAppendQueue::<UTConfig>::new();
    let matching = Some(log_id(1, 1, 10));
    q.push(None, matching);
    q.push(matching, matching);
    let times = spread_sending_times(&mut q);
    let first = q.drain_acked(&matching);
    let empty = q.is_empty();
    assert!(empty);
    q.push(matching, matching);
    let sending_time = times[1] + std::time::Duration::from_secs(1);
    q.queue[0].sending_time = sending_time;
    let empty = q.is_empty();
    assert!(!empty);
    let second = q.drain_acked(&matching);
    let third = q.drain_acked(&matching);
    let actual = [first, second, third];
    let expected = [Some(times[0]); 3];
    assert_eq!(actual, expected);
}

#[test]
fn test_drain_acked_all() {
    let mut q = InflightAppendQueue::<UTConfig>::new();
    q.push(None, Some(log_id(1, 1, 5)));
    q.push(Some(log_id(1, 1, 5)), Some(log_id(1, 1, 10)));
    let times = spread_sending_times(&mut q);

    let matching = Some(log_id(1, 1, 10));
    let actual = q.drain_acked(&matching);
    assert_eq!(actual, Some(times[1]));
    // A transport must not acknowledge beyond its input; retain the conservative fallback.
    let matching = Some(log_id(1, 1, 20));
    let actual = q.drain_acked(&matching);
    assert_eq!(actual, Some(times[1]));
    let empty = q.is_empty();
    assert!(empty);
}

#[test]
fn test_drain_acked_empty_queue() {
    let mut q = InflightAppendQueue::<UTConfig>::new();
    let matching = Some(log_id(1, 1, 10));
    let actual = q.drain_acked(&matching);
    assert_eq!(actual, None);
}

#[test]
fn test_drain_acked_with_none_log_id() {
    let mut q = InflightAppendQueue::<UTConfig>::new();
    q.push(None, None);
    q.push(None, None);
    q.push(None, Some(log_id(1, 1, 5)));

    let times = spread_sending_times(&mut q);
    let actual = q.drain_acked(&None);
    assert_eq!(actual, Some(times[0]));
    let empty = q.is_empty();
    assert!(!empty);
    let matching = Some(log_id(1, 1, 5));
    let actual = q.drain_acked(&matching);
    assert_eq!(actual, Some(times[2]));
    let actual = q.drain_acked(&None);
    assert_eq!(actual, None);
    let empty = q.is_empty();
    assert!(empty);
}
