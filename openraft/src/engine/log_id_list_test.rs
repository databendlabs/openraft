use crate::engine::LogIdList;
use crate::LeaderId;
use crate::LogId;

#[test]
fn test_log_id_list_extend_from_same_leader() -> anyhow::Result<()> {
    let mut ids = LogIdList::<u64>::default();

    // Extend one log id to an empty LogIdList: Just store it directly

    ids.extend_from_same_leader(&[log_id(1, 2)]);
    assert_eq!(vec![log_id(1, 2)], ids.key_log_ids());

    // Extend two log ids that are adjacent to the last stored one.
    // It should append only one log id as the new ending log id.

    ids.extend_from_same_leader(&[
        log_id(1, 3), //
        log_id(1, 4),
    ]);
    assert_eq!(
        vec![
            log_id(1, 2), //
            log_id(1, 4)
        ],
        ids.key_log_ids(),
        "same leader as the last"
    );

    // Extend 3 log id with new leader id.
    // It should just store every log id for each leader, plus one last-log-id.

    ids.extend_from_same_leader(&[
        log_id(2, 5), //
        log_id(2, 6),
        log_id(2, 7),
    ]);
    assert_eq!(
        vec![
            log_id(1, 2), //
            log_id(2, 5),
            log_id(2, 7)
        ],
        ids.key_log_ids(),
        "different leader as the last"
    );

    Ok(())
}

#[test]
fn test_log_id_list_append() -> anyhow::Result<()> {
    let mut ids = LogIdList::<u64>::default();

    // Append log id one by one, check the internally constructed `key_log_id` as expected.

    let cases = vec![
        (log_id(1, 2), vec![log_id(1, 2)]), //
        (log_id(1, 3), vec![log_id(1, 2), log_id(1, 3)]),
        (log_id(1, 4), vec![log_id(1, 2), log_id(1, 4)]),
        (log_id(2, 5), vec![log_id(1, 2), log_id(2, 5)]),
        (log_id(2, 7), vec![log_id(1, 2), log_id(2, 5), log_id(2, 7)]),
        (log_id(2, 9), vec![log_id(1, 2), log_id(2, 5), log_id(2, 9)]),
    ];

    for (new_log_id, want) in cases {
        ids.append(new_log_id);
        assert_eq!(want, ids.key_log_ids());
    }

    Ok(())
}

#[test]
fn test_log_id_list_purge() -> anyhow::Result<()> {
    // Append log id one by one, check the internally constructed `key_log_id` as expected.

    let cases = vec![
        //
        (log_id(2, 1), vec![
            log_id(2, 2),
            log_id(3, 3),
            log_id(6, 6),
            log_id(9, 9),
            log_id(9, 11),
        ]),
        (log_id(2, 2), vec![
            log_id(2, 2),
            log_id(3, 3),
            log_id(6, 6),
            log_id(9, 9),
            log_id(9, 11),
        ]),
        (log_id(3, 3), vec![
            log_id(3, 3),
            log_id(6, 6),
            log_id(9, 9),
            log_id(9, 11),
        ]),
        (log_id(3, 4), vec![
            log_id(3, 4),
            log_id(6, 6),
            log_id(9, 9),
            log_id(9, 11),
        ]),
        (log_id(3, 5), vec![
            log_id(3, 5),
            log_id(6, 6),
            log_id(9, 9),
            log_id(9, 11),
        ]),
        (log_id(6, 6), vec![log_id(6, 6), log_id(9, 9), log_id(9, 11)]),
        (log_id(6, 7), vec![log_id(6, 7), log_id(9, 9), log_id(9, 11)]),
        (log_id(6, 8), vec![log_id(6, 8), log_id(9, 9), log_id(9, 11)]),
        (log_id(9, 9), vec![log_id(9, 9), log_id(9, 11)]),
        (log_id(9, 10), vec![log_id(9, 10), log_id(9, 11)]),
        (log_id(9, 11), vec![log_id(9, 11)]),
        (log_id(9, 12), vec![log_id(9, 12)]),
        (log_id(10, 12), vec![log_id(10, 12)]),
    ];

    for (upto, want) in cases {
        let mut ids = LogIdList::<u64>::new(vec![
            log_id(2, 2), // force multi line
            log_id(3, 3),
            log_id(6, 6),
            log_id(9, 9),
            log_id(9, 11),
        ]);

        ids.purge(&upto);
        assert_eq!(want, ids.key_log_ids(), "purge upto: {}", upto);
    }

    Ok(())
}

#[test]
fn test_log_id_list_get_log_id() -> anyhow::Result<()> {
    // Get log id from empty list always returns `None`.

    let ids = LogIdList::<u64>::default();

    assert!(ids.get(0).is_none());
    assert!(ids.get(1).is_none());
    assert!(ids.get(2).is_none());

    // Get log id that is a key log id or not.

    let ids = LogIdList::<u64>::new(vec![
        log_id(1, 1),
        log_id(1, 2),
        log_id(3, 3),
        log_id(5, 6),
        log_id(7, 8),
        log_id(7, 10),
    ]);

    assert_eq!(None, ids.get(0));
    assert_eq!(Some(log_id(1, 1)), ids.get(1));
    assert_eq!(Some(log_id(1, 2)), ids.get(2));
    assert_eq!(Some(log_id(3, 3)), ids.get(3));
    assert_eq!(Some(log_id(3, 4)), ids.get(4));
    assert_eq!(Some(log_id(3, 5)), ids.get(5));
    assert_eq!(Some(log_id(5, 6)), ids.get(6));
    assert_eq!(Some(log_id(5, 7)), ids.get(7));
    assert_eq!(Some(log_id(7, 8)), ids.get(8));
    assert_eq!(Some(log_id(7, 9)), ids.get(9));
    assert_eq!(Some(log_id(7, 10)), ids.get(10));
    assert_eq!(None, ids.get(11));

    Ok(())
}
fn log_id(term: u64, index: u64) -> LogId<u64> {
    LogId::<u64> {
        leader_id: LeaderId { term, node_id: 1 },
        index,
    }
}
