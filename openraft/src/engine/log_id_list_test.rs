use std::collections::HashMap;

use crate::RaftLogReader;
use crate::engine::LogIdList;
use crate::engine::leader_log_ids::LeaderLogIds;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::entry::RaftEntry;
use crate::log_id::raft_log_id_ext::RaftLogIdExt;
use crate::type_config::alias::LogIdOf;

/// Mock RaftLogReader for testing get_key_log_ids
struct MockLogReader {
    logs: HashMap<u64, LogIdOf<UTConfig>>,
}

impl MockLogReader {
    fn new(logs: Vec<LogIdOf<UTConfig>>) -> Self {
        let mut map = HashMap::new();
        for log_id in logs {
            map.insert(log_id.index(), log_id);
        }
        Self { logs: map }
    }
}

impl RaftLogReader<UTConfig> for MockLogReader {
    async fn try_get_log_entries<RNG>(
        &mut self,
        range: RNG,
    ) -> Result<Vec<<UTConfig as crate::RaftTypeConfig>::Entry>, std::io::Error>
    where
        RNG: std::ops::RangeBounds<u64> + Clone + std::fmt::Debug + crate::OptionalSend,
    {
        use std::ops::Bound;

        let start = match range.start_bound() {
            Bound::Included(&s) => s,
            Bound::Excluded(&s) => s + 1,
            Bound::Unbounded => 0,
        };

        let end = match range.end_bound() {
            Bound::Included(&e) => e + 1,
            Bound::Excluded(&e) => e,
            Bound::Unbounded => u64::MAX,
        };

        let mut entries = Vec::new();
        for i in start..end {
            if let Some(log_id) = self.logs.get(&i) {
                // Create a blank entry with this log_id
                entries.push(crate::Entry::new_blank(*log_id));
            }
        }
        Ok(entries)
    }

    async fn read_vote(&mut self) -> Result<Option<<UTConfig as crate::RaftTypeConfig>::Vote>, std::io::Error> {
        Ok(None)
    }
}

#[test]
fn test_log_id_list_extend_from_same_leader() -> anyhow::Result<()> {
    let mut ids = LogIdList::<UTConfig>::default();

    // Extend one log id to an empty LogIdList: Just store it directly

    ids.extend_from_same_leader([log_id(1, 1, 2)]);
    assert_eq!(vec![log_id(1, 1, 2)], ids.key_log_ids());

    // Extend two log ids that are adjacent to the last stored one.
    // Same leader: only keep the last log id.

    ids.extend_from_same_leader([
        log_id(1, 1, 3), //
        log_id(1, 1, 4),
    ]);
    assert_eq!(
        vec![log_id(1, 1, 4)],
        ids.key_log_ids(),
        "same leader as the last, replace with last"
    );

    // Extend 3 log id with new leader id.
    // Different leader: push new entry (last log of new leader).

    ids.extend_from_same_leader([
        log_id(2, 1, 5), //
        log_id(2, 1, 6),
        log_id(2, 1, 7),
    ]);
    assert_eq!(
        vec![
            log_id(1, 1, 4), //
            log_id(2, 1, 7)
        ],
        ids.key_log_ids(),
        "different leader as the last"
    );

    Ok(())
}

#[test]
fn test_log_id_list_extend() -> anyhow::Result<()> {
    let mut ids = LogIdList::<UTConfig>::default();

    // Extend one log id to an empty LogIdList: Just store it directly

    ids.extend([log_id(1, 1, 2)]);
    assert_eq!(vec![log_id(1, 1, 2)], ids.key_log_ids());

    // Extend two log ids that are adjacent to the last stored one.
    // Same leader: replace with the last.

    ids.extend([
        log_id(1, 1, 3), //
        log_id(1, 1, 4),
    ]);
    assert_eq!(vec![log_id(1, 1, 4)], ids.key_log_ids(), "same leader as the last");

    // Extend 3 log id with different leader id.
    // First one (1,1,5) same leader -> replace: [log_id(1,1,5)]
    // Second (2,1,6) different leader -> push: [log_id(1,1,5), log_id(2,1,6)]
    // Third (2,1,7) same leader -> replace: [log_id(1,1,5), log_id(2,1,7)]

    ids.extend([
        log_id(1, 1, 5), //
        log_id(2, 1, 6),
        log_id(2, 1, 7),
    ]);
    assert_eq!(
        vec![
            log_id(1, 1, 5), //
            log_id(2, 1, 7)
        ],
        ids.key_log_ids(),
        "last 2 have the same leaders"
    );

    // Extend 3 log id with different leader id.
    // (2,1,8) same leader -> replace: [log_id(1,1,5), log_id(2,1,8)]
    // (2,1,9) same leader -> replace: [log_id(1,1,5), log_id(2,1,9)]
    // (3,1,10) different leader -> push: [log_id(1,1,5), log_id(2,1,9), log_id(3,1,10)]

    ids.extend([
        log_id(2, 1, 8), //
        log_id(2, 1, 9),
        log_id(3, 1, 10),
    ]);
    assert_eq!(
        vec![
            log_id(1, 1, 5), //
            log_id(2, 1, 9),
            log_id(3, 1, 10),
        ],
        ids.key_log_ids(),
        "last 2 have different leaders"
    );

    Ok(())
}

#[test]
fn test_log_id_list_append() -> anyhow::Result<()> {
    let mut ids = LogIdList::<UTConfig>::default();

    // Append log id one by one, check the internally constructed `key_log_id` as expected.
    // With last-per-leader: same leader replaces, different leader pushes.

    let cases = vec![
        (log_id(1, 1, 2), vec![log_id(1, 1, 2)]),
        (log_id(1, 1, 3), vec![log_id(1, 1, 3)]), // same leader, replace
        (log_id(1, 1, 4), vec![log_id(1, 1, 4)]), // same leader, replace
        (log_id(2, 1, 5), vec![log_id(1, 1, 4), log_id(2, 1, 5)]), // different leader, push
        (log_id(2, 1, 7), vec![log_id(1, 1, 4), log_id(2, 1, 7)]), // same leader, replace
        (log_id(2, 1, 9), vec![log_id(1, 1, 4), log_id(2, 1, 9)]), // same leader, replace
    ];

    for (new_log_id, want) in cases {
        ids.append(new_log_id);
        assert_eq!(want, ids.key_log_ids());
    }

    Ok(())
}

#[test]
fn test_log_id_list_truncate() -> anyhow::Result<()> {
    // Sample data for test (last-per-leader format)
    // With purged=None, first log is at index 0
    // Represents logs:
    // - Leader 2: indices 0-2
    // - Leader 3: indices 3-5
    // - Leader 6: indices 6-8
    // - Leader 9: indices 9-11
    let make_ids = || {
        LogIdList::<UTConfig>::new(None, vec![
            log_id(2, 1, 2),  // last of leader 2
            log_id(3, 1, 5),  // last of leader 3
            log_id(6, 1, 8),  // last of leader 6
            log_id(9, 1, 11), // last of leader 9
        ])
    };

    // truncate(at) removes logs from index `at` onwards, keeps 0..(at-1)
    let cases = vec![
        (0, vec![]),                                                  // keep nothing
        (1, vec![log_id(2, 1, 0)]),                                   // keep 0
        (2, vec![log_id(2, 1, 1)]),                                   // keep 0-1
        (3, vec![log_id(2, 1, 2)]),                                   // keep 0-2
        (4, vec![log_id(2, 1, 2), log_id(3, 1, 3)]),                  // keep 0-3
        (5, vec![log_id(2, 1, 2), log_id(3, 1, 4)]),                  // keep 0-4
        (6, vec![log_id(2, 1, 2), log_id(3, 1, 5)]),                  // keep 0-5
        (7, vec![log_id(2, 1, 2), log_id(3, 1, 5), log_id(6, 1, 6)]), // keep 0-6
        (8, vec![log_id(2, 1, 2), log_id(3, 1, 5), log_id(6, 1, 7)]), // keep 0-7
        (9, vec![log_id(2, 1, 2), log_id(3, 1, 5), log_id(6, 1, 8)]), // keep 0-8
        (10, vec![
            log_id(2, 1, 2),
            log_id(3, 1, 5),
            log_id(6, 1, 8),
            log_id(9, 1, 9),
        ]), // keep 0-9
        (11, vec![
            log_id(2, 1, 2),
            log_id(3, 1, 5),
            log_id(6, 1, 8),
            log_id(9, 1, 10),
        ]), // keep 0-10
        (12, vec![
            log_id(2, 1, 2),
            log_id(3, 1, 5),
            log_id(6, 1, 8),
            log_id(9, 1, 11),
        ]), // keep all
        (13, vec![
            log_id(2, 1, 2),
            log_id(3, 1, 5),
            log_id(6, 1, 8),
            log_id(9, 1, 11),
        ]), // keep all
    ];

    for (at, want) in cases {
        let mut ids = make_ids();

        ids.truncate(at);
        assert_eq!(want, ids.key_log_ids(), "truncate since: [{}, +oo)", at);
    }

    Ok(())
}

#[test]
fn test_log_id_list_purge() -> anyhow::Result<()> {
    // Purge on an empty log id list:
    {
        let mut ids = LogIdList::<UTConfig>::new(None, vec![]);
        ids.purge(&log_id(2, 1, 2));
        assert_eq!(Some(&log_id(2, 1, 2)), ids.purged());
        assert!(ids.key_log_ids().is_empty());
    }

    // Sample data for test (last-per-leader format)
    // Represents logs:
    // - Leader 2: index 2
    // - Leader 3: indices 3-5
    // - Leader 6: indices 6-8
    // - Leader 9: indices 9-11
    let make_ids = || {
        LogIdList::<UTConfig>::new(None, vec![
            log_id(2, 1, 2),  // last of leader 2
            log_id(3, 1, 5),  // last of leader 3
            log_id(6, 1, 8),  // last of leader 6
            log_id(9, 1, 11), // last of leader 9
        ])
    };

    // Test cases: (purge_upto, expected_purged, expected_key_log_ids)
    let cases: Vec<(_, Option<_>, Vec<_>)> = vec![
        // Purge before first log - still sets purged field
        (log_id(2, 1, 1), Some(log_id(2, 1, 1)), vec![
            log_id(2, 1, 2),
            log_id(3, 1, 5),
            log_id(6, 1, 8),
            log_id(9, 1, 11),
        ]),
        // Purge exactly at first entry (index 2, leader 2's last)
        (log_id(2, 1, 2), Some(log_id(2, 1, 2)), vec![
            log_id(3, 1, 5),
            log_id(6, 1, 8),
            log_id(9, 1, 11),
        ]),
        // Purge within leader 3's range
        (log_id(3, 1, 3), Some(log_id(3, 1, 3)), vec![
            log_id(3, 1, 5),
            log_id(6, 1, 8),
            log_id(9, 1, 11),
        ]),
        (log_id(3, 1, 4), Some(log_id(3, 1, 4)), vec![
            log_id(3, 1, 5),
            log_id(6, 1, 8),
            log_id(9, 1, 11),
        ]),
        // Purge at leader 3's last index
        (log_id(3, 1, 5), Some(log_id(3, 1, 5)), vec![
            log_id(6, 1, 8),
            log_id(9, 1, 11),
        ]),
        // Purge within leader 6's range
        (log_id(6, 1, 6), Some(log_id(6, 1, 6)), vec![
            log_id(6, 1, 8),
            log_id(9, 1, 11),
        ]),
        (log_id(6, 1, 7), Some(log_id(6, 1, 7)), vec![
            log_id(6, 1, 8),
            log_id(9, 1, 11),
        ]),
        // Purge at leader 6's last index
        (log_id(6, 1, 8), Some(log_id(6, 1, 8)), vec![log_id(9, 1, 11)]),
        // Purge within leader 9's range
        (log_id(9, 1, 9), Some(log_id(9, 1, 9)), vec![log_id(9, 1, 11)]),
        (log_id(9, 1, 10), Some(log_id(9, 1, 10)), vec![log_id(9, 1, 11)]),
        // Purge at/beyond last log
        (log_id(9, 1, 11), Some(log_id(9, 1, 11)), vec![]),
        (log_id(9, 1, 12), Some(log_id(9, 1, 12)), vec![]),
        (log_id(10, 1, 12), Some(log_id(10, 1, 12)), vec![]),
    ];

    for (upto, want_purged, want_key_log_ids) in cases {
        let mut ids = make_ids();

        ids.purge(&upto);
        assert_eq!(
            want_purged.as_ref(),
            ids.purged(),
            "purge upto: {} - purged field",
            upto
        );
        assert_eq!(
            want_key_log_ids,
            ids.key_log_ids(),
            "purge upto: {} - key_log_ids",
            upto
        );
    }

    Ok(())
}

#[test]
fn test_log_id_list_get_log_id() -> anyhow::Result<()> {
    // Get log id from empty list always returns `None`.

    let ids = LogIdList::<UTConfig>::default();

    assert!(ids.get(0).is_none());
    assert!(ids.get(1).is_none());
    assert!(ids.get(2).is_none());

    // Get log id that is a key log id or not.
    // Last-per-leader format:
    // - Leader 1: indices 0-2 (last at 2)
    // - Leader 3: indices 3-5 (last at 5)
    // - Leader 5: indices 6-7 (last at 7)
    // - Leader 7: indices 8-10 (last at 10)

    let ids = LogIdList::<UTConfig>::new(None, vec![
        log_id(1, 1, 2),  // last of leader 1
        log_id(3, 1, 5),  // last of leader 3
        log_id(5, 1, 7),  // last of leader 5
        log_id(7, 1, 10), // last of leader 7
    ]);

    assert_eq!(Some(log_id(1, 1, 0)), ids.get(0));
    assert_eq!(Some(log_id(1, 1, 1)), ids.get(1));
    assert_eq!(Some(log_id(1, 1, 2)), ids.get(2));
    assert_eq!(Some(log_id(3, 1, 3)), ids.get(3));
    assert_eq!(Some(log_id(3, 1, 4)), ids.get(4));
    assert_eq!(Some(log_id(3, 1, 5)), ids.get(5));
    assert_eq!(Some(log_id(5, 1, 6)), ids.get(6));
    assert_eq!(Some(log_id(5, 1, 7)), ids.get(7));
    assert_eq!(Some(log_id(7, 1, 8)), ids.get(8));
    assert_eq!(Some(log_id(7, 1, 9)), ids.get(9));
    assert_eq!(Some(log_id(7, 1, 10)), ids.get(10));
    assert_eq!(None, ids.get(11));

    Ok(())
}

#[test]
fn test_log_id_list_by_last_leader() -> anyhow::Result<()> {
    // len == 0
    let ids = LogIdList::<UTConfig>::default();
    assert_eq!(ids.by_last_leader(), None);

    // len == 1, leader's range starts at 0 (no purged)
    // Last-per-leader: [log_id(1,1,1)] means leader 1's last log is at index 1
    // First index is purged.index+1 = 0
    let ids = LogIdList::<UTConfig>::new(None, [log_id(1, 1, 1)]);
    assert_eq!(
        Some(LeaderLogIds::new(*log_id(1, 1, 0).committed_leader_id(), 0, 1)),
        ids.by_last_leader()
    );

    // len == 2, different leaders
    // [log_id(1,1,1), log_id(3,1,3)] means:
    // - Leader 1: indices 0-1 (last at 1)
    // - Leader 3: indices 2-3 (last at 3)
    // by_last_leader returns leader 3's range: first_index = 1+1 = 2, last_index = 3
    let ids = LogIdList::<UTConfig>::new(None, [log_id(1, 1, 1), log_id(3, 1, 3)]);
    assert_eq!(
        Some(LeaderLogIds::new(*log_id(3, 1, 0).committed_leader_id(), 2, 3)),
        ids.by_last_leader()
    );

    // len == 1 with purged
    // purged at index 2, leader 1's last at index 5
    // Leader 1's range: first_index = 2+1 = 3, last_index = 5
    let ids = LogIdList::<UTConfig>::new(Some(log_id(1, 1, 2)), [log_id(1, 1, 5)]);
    assert_eq!(
        Some(LeaderLogIds::new(*log_id(1, 1, 0).committed_leader_id(), 3, 5)),
        ids.by_last_leader()
    );

    // len > 1
    // [log_id(1,1,2), log_id(7,1,10)] means:
    // - Leader 1: indices 0-2 (last at 2)
    // - Leader 7: indices 3-10 (last at 10)
    // by_last_leader returns leader 7's range: first_index = 2+1 = 3, last_index = 10
    let ids = LogIdList::<UTConfig>::new(None, [log_id(1, 1, 2), log_id(7, 1, 10)]);
    assert_eq!(
        Some(LeaderLogIds::new(*log_id(7, 1, 0).committed_leader_id(), 3, 10)),
        ids.by_last_leader()
    );

    Ok(())
}

#[test]
fn test_log_id_list_last() -> anyhow::Result<()> {
    // Test last() which returns key_log_ids.last().or(purged.as_ref())

    // 0 elements, no purged
    let ids = LogIdList::<UTConfig>::default();
    assert_eq!(None, ids.last());

    // 0 elements, with purged
    let ids = LogIdList::<UTConfig>::new(Some(log_id(2, 1, 5)), vec![]);
    assert_eq!(Some(&log_id(2, 1, 5)), ids.last());

    // 1 element, no purged
    let ids = LogIdList::<UTConfig>::new(None, vec![log_id(1, 1, 3)]);
    assert_eq!(Some(&log_id(1, 1, 3)), ids.last());

    // 1 element, with purged
    let ids = LogIdList::<UTConfig>::new(Some(log_id(1, 1, 2)), vec![log_id(1, 1, 5)]);
    assert_eq!(Some(&log_id(1, 1, 5)), ids.last());

    // 2 elements, no purged
    let ids = LogIdList::<UTConfig>::new(None, vec![log_id(1, 1, 3), log_id(2, 1, 6)]);
    assert_eq!(Some(&log_id(2, 1, 6)), ids.last());

    // 2 elements, with purged
    let ids = LogIdList::<UTConfig>::new(Some(log_id(1, 1, 2)), vec![log_id(1, 1, 5), log_id(2, 1, 8)]);
    assert_eq!(Some(&log_id(2, 1, 8)), ids.last());

    Ok(())
}

#[test]
fn test_log_id_list_first() -> anyhow::Result<()> {
    // Test first() which returns first log id
    // Leader comes from key_log_ids[0], index is purged.index + 1 (or 0)

    // 0 elements, no purged
    let ids = LogIdList::<UTConfig>::default();
    assert_eq!(None, ids.first());

    // 0 elements, with purged - returns None (no key_log_ids)
    let ids = LogIdList::<UTConfig>::new(Some(log_id(2, 1, 5)), vec![]);
    assert_eq!(None, ids.first());

    // 1 element, no purged - first index is 0
    let ids = LogIdList::<UTConfig>::new(None, vec![log_id(1, 1, 3)]);
    assert_eq!(Some(log_id(1, 1, 0).to_ref()), ids.first());

    // 1 element, with purged - first index is purged.index + 1 = 3
    let ids = LogIdList::<UTConfig>::new(Some(log_id(1, 1, 2)), vec![log_id(1, 1, 5)]);
    assert_eq!(Some(log_id(1, 1, 3).to_ref()), ids.first());

    // 2 elements, no purged - first index is 0
    let ids = LogIdList::<UTConfig>::new(None, vec![log_id(1, 1, 3), log_id(2, 1, 6)]);
    assert_eq!(Some(log_id(1, 1, 0).to_ref()), ids.first());

    // 2 elements, with purged - first index is purged.index + 1 = 3
    let ids = LogIdList::<UTConfig>::new(Some(log_id(1, 1, 2)), vec![log_id(1, 1, 5), log_id(2, 1, 8)]);
    assert_eq!(Some(log_id(1, 1, 3).to_ref()), ids.first());

    Ok(())
}

#[test]
fn test_log_id_list_ref_at_with_purged() -> anyhow::Result<()> {
    // Test ref_at() accessing various indices when purged is set
    // Setup: purged at 5 (leader 2), key_log_ids = [log_id(2,1,8)]
    // This means: leader 2's logs span indices 6-8

    let ids = LogIdList::<UTConfig>::new(Some(log_id(2, 1, 5)), vec![log_id(2, 1, 8)]);

    // Access before purged - None
    assert_eq!(None, ids.ref_at(4));

    // Access at purged index - returns purged
    assert_eq!(Some(log_id(2, 1, 5).to_ref()), ids.ref_at(5));

    // Access after purged, in range (6, 7)
    assert_eq!(Some(log_id(2, 1, 6).to_ref()), ids.ref_at(6));
    assert_eq!(Some(log_id(2, 1, 7).to_ref()), ids.ref_at(7));

    // Access at last
    assert_eq!(Some(log_id(2, 1, 8).to_ref()), ids.ref_at(8));

    // Access beyond last - None
    assert_eq!(None, ids.ref_at(9));

    // All purged, access purged index
    let ids = LogIdList::<UTConfig>::new(Some(log_id(2, 1, 5)), vec![]);
    assert_eq!(Some(log_id(2, 1, 5).to_ref()), ids.ref_at(5));

    // All purged, access other indices - None
    assert_eq!(None, ids.ref_at(4));
    assert_eq!(None, ids.ref_at(6));

    Ok(())
}

#[test]
fn test_log_id_list_get_with_purged() -> anyhow::Result<()> {
    // Test get() with purged set
    // Setup: purged at 2 (leader 1), key_log_ids = [log_id(1,1,5), log_id(3,1,8)]
    // Logs: leader 1 at 3-5, leader 3 at 6-8

    let ids = LogIdList::<UTConfig>::new(Some(log_id(1, 1, 2)), vec![log_id(1, 1, 5), log_id(3, 1, 8)]);

    // Before purged - None
    assert_eq!(None, ids.get(0));
    assert_eq!(None, ids.get(1));

    // At purged index
    assert_eq!(Some(log_id(1, 1, 2)), ids.get(2));

    // Within leader 1's range (3-5)
    assert_eq!(Some(log_id(1, 1, 3)), ids.get(3));
    assert_eq!(Some(log_id(1, 1, 4)), ids.get(4));
    assert_eq!(Some(log_id(1, 1, 5)), ids.get(5));

    // Within leader 3's range (6-8)
    assert_eq!(Some(log_id(3, 1, 6)), ids.get(6));
    assert_eq!(Some(log_id(3, 1, 7)), ids.get(7));
    assert_eq!(Some(log_id(3, 1, 8)), ids.get(8));

    // Beyond last - None
    assert_eq!(None, ids.get(9));

    Ok(())
}

#[test]
fn test_log_id_list_truncate_with_purged() -> anyhow::Result<()> {
    // Test truncate() when purged is set
    // Setup: purged at 2 (leader 1), key_log_ids = [log_id(1,1,5), log_id(2,1,8)]
    // Logs: purged at 2, leader 1: 3-5, leader 2: 6-8

    let make_ids = || LogIdList::<UTConfig>::new(Some(log_id(1, 1, 2)), vec![log_id(1, 1, 5), log_id(2, 1, 8)]);

    // truncate(3) - keep nothing in key_log_ids (truncate from first available index)
    {
        let mut ids = make_ids();
        ids.truncate(3);
        assert_eq!(Some(&log_id(1, 1, 2)), ids.purged());
        assert!(ids.key_log_ids().is_empty());
    }

    // truncate(4) - keep index 3
    {
        let mut ids = make_ids();
        ids.truncate(4);
        assert_eq!(Some(&log_id(1, 1, 2)), ids.purged());
        assert_eq!(vec![log_id(1, 1, 3)], ids.key_log_ids());
    }

    // truncate(5) - keep indices 3-4
    {
        let mut ids = make_ids();
        ids.truncate(5);
        assert_eq!(Some(&log_id(1, 1, 2)), ids.purged());
        assert_eq!(vec![log_id(1, 1, 4)], ids.key_log_ids());
    }

    // truncate(6) - keep indices 3-5 (all of leader 1)
    {
        let mut ids = make_ids();
        ids.truncate(6);
        assert_eq!(Some(&log_id(1, 1, 2)), ids.purged());
        assert_eq!(vec![log_id(1, 1, 5)], ids.key_log_ids());
    }

    // truncate(7) - keep indices 3-6
    {
        let mut ids = make_ids();
        ids.truncate(7);
        assert_eq!(Some(&log_id(1, 1, 2)), ids.purged());
        assert_eq!(vec![log_id(1, 1, 5), log_id(2, 1, 6)], ids.key_log_ids());
    }

    // truncate(8) - keep indices 3-7
    {
        let mut ids = make_ids();
        ids.truncate(8);
        assert_eq!(Some(&log_id(1, 1, 2)), ids.purged());
        assert_eq!(vec![log_id(1, 1, 5), log_id(2, 1, 7)], ids.key_log_ids());
    }

    // truncate(9) - keep all (indices 3-8)
    {
        let mut ids = make_ids();
        ids.truncate(9);
        assert_eq!(Some(&log_id(1, 1, 2)), ids.purged());
        assert_eq!(vec![log_id(1, 1, 5), log_id(2, 1, 8)], ids.key_log_ids());
    }

    Ok(())
}

#[test]
fn test_log_id_list_append_with_purged() -> anyhow::Result<()> {
    // Test append() when starting with purged set and empty key_log_ids
    // Setup: purged at 2 (leader 1), key_log_ids = []

    let mut ids = LogIdList::<UTConfig>::new(Some(log_id(1, 1, 2)), vec![]);

    // Append same leader as purged - should push (not compare with purged)
    ids.append(log_id(1, 1, 3));
    assert_eq!(Some(&log_id(1, 1, 2)), ids.purged());
    assert_eq!(vec![log_id(1, 1, 3)], ids.key_log_ids());

    // Append same leader again - replaces
    ids.append(log_id(1, 1, 4));
    assert_eq!(vec![log_id(1, 1, 4)], ids.key_log_ids());

    // Append different leader - pushes
    ids.append(log_id(2, 1, 5));
    assert_eq!(vec![log_id(1, 1, 4), log_id(2, 1, 5)], ids.key_log_ids());

    Ok(())
}

#[test]
fn test_log_id_list_append_with_purged_different_leader() -> anyhow::Result<()> {
    // Test append() when purged is Some, key_log_ids is empty, and new log has different leader
    // Setup: purged at 2 (leader 1), key_log_ids = []

    let mut ids = LogIdList::<UTConfig>::new(Some(log_id(1, 1, 2)), vec![]);

    // Append different leader than purged - should push
    ids.append(log_id(2, 1, 3));
    assert_eq!(Some(&log_id(1, 1, 2)), ids.purged());
    assert_eq!(vec![log_id(2, 1, 3)], ids.key_log_ids());

    Ok(())
}

#[test]
fn test_log_id_list_extend_with_purged() -> anyhow::Result<()> {
    // Test extend() when purged is Some and key_log_ids is empty

    // Case 1: Extend with same leader as purged
    {
        let mut ids = LogIdList::<UTConfig>::new(Some(log_id(1, 1, 2)), vec![]);
        ids.extend([log_id(1, 1, 3), log_id(1, 1, 4)]);
        assert_eq!(Some(&log_id(1, 1, 2)), ids.purged());
        assert_eq!(vec![log_id(1, 1, 4)], ids.key_log_ids());
    }

    // Case 2: Extend with different leader than purged
    {
        let mut ids = LogIdList::<UTConfig>::new(Some(log_id(1, 1, 2)), vec![]);
        ids.extend([log_id(2, 1, 3), log_id(2, 1, 4)]);
        assert_eq!(Some(&log_id(1, 1, 2)), ids.purged());
        assert_eq!(vec![log_id(2, 1, 4)], ids.key_log_ids());
    }

    // Case 3: Extend with mixed leaders, starting with same as purged
    {
        let mut ids = LogIdList::<UTConfig>::new(Some(log_id(1, 1, 2)), vec![]);
        ids.extend([log_id(1, 1, 3), log_id(2, 1, 4), log_id(2, 1, 5)]);
        assert_eq!(Some(&log_id(1, 1, 2)), ids.purged());
        assert_eq!(vec![log_id(1, 1, 3), log_id(2, 1, 5)], ids.key_log_ids());
    }

    Ok(())
}

#[test]
fn test_log_id_list_extend_from_same_leader_with_purged() -> anyhow::Result<()> {
    // Test extend_from_same_leader() when purged is Some and key_log_ids is empty

    // Case 1: Extend with same leader as purged
    {
        let mut ids = LogIdList::<UTConfig>::new(Some(log_id(1, 1, 2)), vec![]);
        ids.extend_from_same_leader([log_id(1, 1, 3), log_id(1, 1, 4), log_id(1, 1, 5)]);
        assert_eq!(Some(&log_id(1, 1, 2)), ids.purged());
        assert_eq!(vec![log_id(1, 1, 5)], ids.key_log_ids());
    }

    // Case 2: Extend with different leader than purged
    {
        let mut ids = LogIdList::<UTConfig>::new(Some(log_id(1, 1, 2)), vec![]);
        ids.extend_from_same_leader([log_id(2, 1, 3), log_id(2, 1, 4), log_id(2, 1, 5)]);
        assert_eq!(Some(&log_id(1, 1, 2)), ids.purged());
        assert_eq!(vec![log_id(2, 1, 5)], ids.key_log_ids());
    }

    Ok(())
}

#[test]
fn test_log_id_list_purge_edge_cases() -> anyhow::Result<()> {
    // Additional purge edge cases with 0, 1, 2 elements

    // 0 elements: Purge on empty with prior purge (should update purged)
    {
        let mut ids = LogIdList::<UTConfig>::new(Some(log_id(1, 1, 2)), vec![]);
        ids.purge(&log_id(2, 1, 5));
        assert_eq!(Some(&log_id(2, 1, 5)), ids.purged());
        assert!(ids.key_log_ids().is_empty());
    }

    // 1 element: Purge before the single entry's range
    // Setup: purged=None, key_log_ids=[log_id(1,1,3)] means logs at 0-3
    {
        let mut ids = LogIdList::<UTConfig>::new(None, vec![log_id(1, 1, 3)]);
        ids.purge(&log_id(1, 1, 1));
        assert_eq!(Some(&log_id(1, 1, 1)), ids.purged());
        assert_eq!(vec![log_id(1, 1, 3)], ids.key_log_ids());
    }

    // 1 element: Purge at the single entry's last index
    {
        let mut ids = LogIdList::<UTConfig>::new(None, vec![log_id(1, 1, 3)]);
        ids.purge(&log_id(1, 1, 3));
        assert_eq!(Some(&log_id(1, 1, 3)), ids.purged());
        assert!(ids.key_log_ids().is_empty());
    }

    // 1 element: Purge beyond the single entry
    {
        let mut ids = LogIdList::<UTConfig>::new(None, vec![log_id(1, 1, 3)]);
        ids.purge(&log_id(1, 1, 5));
        assert_eq!(Some(&log_id(1, 1, 5)), ids.purged());
        assert!(ids.key_log_ids().is_empty());
    }

    // 2 elements: Purge within first leader's range
    // [log_id(1,1,3), log_id(2,1,6)] means leader 1: 0-3, leader 2: 4-6
    {
        let mut ids = LogIdList::<UTConfig>::new(None, vec![log_id(1, 1, 3), log_id(2, 1, 6)]);
        ids.purge(&log_id(1, 1, 2));
        assert_eq!(Some(&log_id(1, 1, 2)), ids.purged());
        assert_eq!(vec![log_id(1, 1, 3), log_id(2, 1, 6)], ids.key_log_ids());
    }

    // 2 elements: Purge at boundary between leaders
    {
        let mut ids = LogIdList::<UTConfig>::new(None, vec![log_id(1, 1, 3), log_id(2, 1, 6)]);
        ids.purge(&log_id(1, 1, 3));
        assert_eq!(Some(&log_id(1, 1, 3)), ids.purged());
        assert_eq!(vec![log_id(2, 1, 6)], ids.key_log_ids());
    }

    // 2 elements: Purge within second leader's range
    {
        let mut ids = LogIdList::<UTConfig>::new(None, vec![log_id(1, 1, 3), log_id(2, 1, 6)]);
        ids.purge(&log_id(2, 1, 5));
        assert_eq!(Some(&log_id(2, 1, 5)), ids.purged());
        assert_eq!(vec![log_id(2, 1, 6)], ids.key_log_ids());
    }

    Ok(())
}

#[test]
fn test_log_id_list_by_last_leader_edge_cases() -> anyhow::Result<()> {
    // Additional edge cases for by_last_leader()

    // 0 elements, with purged - should return purged info
    // purged at 5 means the last leader's range is just index 5
    let ids = LogIdList::<UTConfig>::new(Some(log_id(1, 1, 5)), vec![]);
    assert_eq!(
        Some(LeaderLogIds::new(*log_id(1, 1, 0).committed_leader_id(), 5, 5)),
        ids.by_last_leader()
    );

    // 1 element, purged same leader
    // purged at 2, leader 1's last at 5 -> range 3-5
    let ids = LogIdList::<UTConfig>::new(Some(log_id(1, 1, 2)), vec![log_id(1, 1, 5)]);
    assert_eq!(
        Some(LeaderLogIds::new(*log_id(1, 1, 0).committed_leader_id(), 3, 5)),
        ids.by_last_leader()
    );

    // 1 element, purged different leader
    // purged at 2, leader 2's last at 5 -> range 3-5
    let ids = LogIdList::<UTConfig>::new(Some(log_id(1, 1, 2)), vec![log_id(2, 1, 5)]);
    assert_eq!(
        Some(LeaderLogIds::new(*log_id(2, 1, 0).committed_leader_id(), 3, 5)),
        ids.by_last_leader()
    );

    // 2 elements, with purged
    // purged at 2, leader 1 at 3-4, leader 2 at 5-7 -> last leader range 5-7
    let ids = LogIdList::<UTConfig>::new(Some(log_id(1, 1, 2)), vec![log_id(1, 1, 4), log_id(2, 1, 7)]);
    assert_eq!(
        Some(LeaderLogIds::new(*log_id(2, 1, 0).committed_leader_id(), 5, 7)),
        ids.by_last_leader()
    );

    Ok(())
}

#[tokio::test]
async fn test_get_key_log_ids_single_log() -> anyhow::Result<()> {
    // Single log: [(1,0)]
    let logs = vec![log_id(1, 1, 0)];
    let mut reader = MockLogReader::new(logs);

    let result = LogIdList::get_key_log_ids(log_id(1, 1, 0)..=log_id(1, 1, 0), &mut reader).await?;

    assert_eq!(vec![log_id(1, 1, 0)], result);
    Ok(())
}

#[tokio::test]
async fn test_get_key_log_ids_all_same_leader() -> anyhow::Result<()> {
    // All same leader: [(1,0), (1,1), (1,2)]
    // Expected: [(1,2)] - last of leader 1
    // This tests Case AA with empty res (None branch)
    let logs = vec![log_id(1, 1, 0), log_id(1, 1, 1), log_id(1, 1, 2)];
    let mut reader = MockLogReader::new(logs);

    let result = LogIdList::get_key_log_ids(log_id(1, 1, 0)..=log_id(1, 1, 2), &mut reader).await?;

    assert_eq!(vec![log_id(1, 1, 2)], result);
    Ok(())
}

#[tokio::test]
async fn test_get_key_log_ids_two_adjacent_leaders() -> anyhow::Result<()> {
    // Two adjacent logs with different leaders: [(1,0), (2,1)]
    // Expected: [(1,0), (2,1)]
    let logs = vec![log_id(1, 1, 0), log_id(2, 1, 1)];
    let mut reader = MockLogReader::new(logs);

    let result = LogIdList::get_key_log_ids(log_id(1, 1, 0)..=log_id(2, 1, 1), &mut reader).await?;

    assert_eq!(vec![log_id(1, 1, 0), log_id(2, 1, 1)], result);
    Ok(())
}

#[tokio::test]
async fn test_get_key_log_ids_case_aac() -> anyhow::Result<()> {
    // Case AAC: first.leader == mid.leader != last.leader
    // Logs: [(1,0), (1,1), (2,2)]
    // Expected: [(1,1), (2,2)]
    let logs = vec![log_id(1, 1, 0), log_id(1, 1, 1), log_id(2, 1, 2)];
    let mut reader = MockLogReader::new(logs);

    let result = LogIdList::get_key_log_ids(log_id(1, 1, 0)..=log_id(2, 1, 2), &mut reader).await?;

    assert_eq!(vec![log_id(1, 1, 1), log_id(2, 1, 2)], result);
    Ok(())
}

#[tokio::test]
async fn test_get_key_log_ids_case_acc() -> anyhow::Result<()> {
    // Case ACC: first.leader != mid.leader == last.leader
    // This is the bug case! Must search both halves.
    // Logs: [(1,0), (2,1), (2,2), (2,3)]
    // Expected: [(1,0), (2,3)]
    let logs = vec![log_id(1, 1, 0), log_id(2, 1, 1), log_id(2, 1, 2), log_id(2, 1, 3)];
    let mut reader = MockLogReader::new(logs);

    let result = LogIdList::get_key_log_ids(log_id(1, 1, 0)..=log_id(2, 1, 3), &mut reader).await?;

    assert_eq!(vec![log_id(1, 1, 0), log_id(2, 1, 3)], result);
    Ok(())
}

#[tokio::test]
async fn test_get_key_log_ids_case_abc() -> anyhow::Result<()> {
    // Case ABC: first.leader != mid.leader != last.leader
    // Logs: [(1,0), (2,1), (3,2)]
    // Expected: [(1,0), (2,1), (3,2)]
    let logs = vec![log_id(1, 1, 0), log_id(2, 1, 1), log_id(3, 1, 2)];
    let mut reader = MockLogReader::new(logs);

    let result = LogIdList::get_key_log_ids(log_id(1, 1, 0)..=log_id(3, 1, 2), &mut reader).await?;

    assert_eq!(vec![log_id(1, 1, 0), log_id(2, 1, 1), log_id(3, 1, 2)], result);
    Ok(())
}

#[tokio::test]
async fn test_get_key_log_ids_many_same_leader() -> anyhow::Result<()> {
    // Many logs from same leader: [(1,0), (2,1), (2,2), (2,3), (2,4)]
    // Expected: [(1,0), (2,4)] - last of each leader
    // This tests Case ACC with multiple logs in right half
    let logs = vec![
        log_id(1, 1, 0),
        log_id(2, 1, 1),
        log_id(2, 1, 2),
        log_id(2, 1, 3),
        log_id(2, 1, 4),
    ];
    let mut reader = MockLogReader::new(logs);

    let result = LogIdList::get_key_log_ids(log_id(1, 1, 0)..=log_id(2, 1, 4), &mut reader).await?;

    assert_eq!(vec![log_id(1, 1, 0), log_id(2, 1, 4)], result);
    Ok(())
}

#[tokio::test]
async fn test_get_key_log_ids_three_leaders() -> anyhow::Result<()> {
    // Three leaders with varying counts: [(1,0), (2,1), (2,2), (2,3), (3,4), (3,5)]
    // Expected: [(1,0), (2,3), (3,5)]
    let logs = vec![
        log_id(1, 1, 0),
        log_id(2, 1, 1),
        log_id(2, 1, 2),
        log_id(2, 1, 3),
        log_id(3, 1, 4),
        log_id(3, 1, 5),
    ];
    let mut reader = MockLogReader::new(logs);

    let result = LogIdList::get_key_log_ids(log_id(1, 1, 0)..=log_id(3, 1, 5), &mut reader).await?;

    assert_eq!(vec![log_id(1, 1, 0), log_id(2, 1, 3), log_id(3, 1, 5)], result);
    Ok(())
}

#[tokio::test]
async fn test_get_key_log_ids_long_same_leader() -> anyhow::Result<()> {
    // Many logs from same leader: [(1,0), (1,1), ..., (1,9)]
    // Expected: [(1,9)]
    // Stresses binary search with many subdivisions
    let logs: Vec<_> = (0..10).map(|i| log_id(1, 1, i)).collect();
    let mut reader = MockLogReader::new(logs);

    let result = LogIdList::get_key_log_ids(log_id(1, 1, 0)..=log_id(1, 1, 9), &mut reader).await?;

    assert_eq!(vec![log_id(1, 1, 9)], result);
    Ok(())
}
