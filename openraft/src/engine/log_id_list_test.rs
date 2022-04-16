use crate::engine::LogIdList;
use crate::LeaderId;
use crate::LogId;

#[test]
fn test_log_id_list_extend_from_same_leader() -> anyhow::Result<()> {
    let log_id = |t, i| LogId::<u64> {
        leader_id: LeaderId { term: t, node_id: 1 },
        index: i,
    };

    let mut ids = LogIdList::<u64>::default();

    ids.extend_from_same_leader(&[log_id(1, 2)]);
    assert_eq!(vec![log_id(1, 2)], ids.key_log_ids());

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
    let log_id = |t, i| LogId::<u64> {
        leader_id: LeaderId { term: t, node_id: 1 },
        index: i,
    };

    let mut ids = LogIdList::<u64>::default();

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
fn test_log_id_list_get_log_id() -> anyhow::Result<()> {
    let log_id = |t, i| LogId::<u64> {
        leader_id: LeaderId { term: t, node_id: 1 },
        index: i,
    };

    let ids = LogIdList::<u64>::default();

    assert!(ids.get(0).is_none());
    assert!(ids.get(1).is_none());
    assert!(ids.get(2).is_none());

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
