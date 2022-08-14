use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::LeaderId;
use crate::LogId;

fn log_id(term: u64, index: u64) -> LogId<u64> {
    LogId::<u64> {
        leader_id: LeaderId { term, node_id: 0 },
        index,
    }
}

fn eng() -> Engine<u64, ()> {
    let mut eng = Engine::default();
    eng.state.log_ids = LogIdList::new(vec![
        //
        log_id(0, 0),
        log_id(1, 1),
        log_id(3, 3),
        log_id(5, 5),
    ]);
    eng
}

#[test]
fn test_calc_purge_upto() -> anyhow::Result<()> {
    // last_purged_log_id, committed, max_keep, want
    let cases = vec![
        //
        (None, None, 0, None),
        (None, None, 1, None),
        //
        (None, Some(log_id(1, 1)), 0, Some(log_id(1, 1))),
        (None, Some(log_id(1, 1)), 1, None),
        (None, Some(log_id(1, 1)), 2, None),
        //
        (Some(log_id(0, 0)), Some(log_id(1, 1)), 0, Some(log_id(1, 1))),
        (Some(log_id(0, 0)), Some(log_id(1, 1)), 1, None),
        (Some(log_id(0, 0)), Some(log_id(1, 1)), 2, None),
        //
        (None, Some(log_id(3, 4)), 0, Some(log_id(3, 4))),
        (None, Some(log_id(3, 4)), 1, Some(log_id(3, 3))),
        (None, Some(log_id(3, 4)), 2, Some(log_id(1, 2))),
        (None, Some(log_id(3, 4)), 3, Some(log_id(1, 1))),
        (None, Some(log_id(3, 4)), 4, None),
        (None, Some(log_id(3, 4)), 5, None),
        //
        (Some(log_id(1, 2)), Some(log_id(3, 4)), 0, Some(log_id(3, 4))),
        (Some(log_id(1, 2)), Some(log_id(3, 4)), 1, Some(log_id(3, 3))),
        (Some(log_id(1, 2)), Some(log_id(3, 4)), 2, None),
        (Some(log_id(1, 2)), Some(log_id(3, 4)), 3, None),
        (Some(log_id(1, 2)), Some(log_id(3, 4)), 4, None),
        (Some(log_id(1, 2)), Some(log_id(3, 4)), 5, None),
    ];

    for (last_purged, committed, max_keep, want) in cases {
        let mut eng = eng();
        eng.config.keep_unsnapshoted_log = false;
        eng.config.max_applied_log_to_keep = max_keep;
        eng.config.purge_batch_size = 1;

        if let Some(last_purged) = last_purged {
            eng.state.log_ids.purge(&last_purged);
        }
        eng.state.committed = committed;
        let got = eng.calc_purge_upto();

        assert_eq!(
            want, got,
            "case: last_purged: {:?}, last_applied: {:?}, max_keep: {}",
            last_purged, committed, max_keep
        );
    }

    Ok(())
}

#[test]
// in this test, keep_unsnapshoted_log is set to true.
// logs being purged should at most the last that was in the snapshot.
fn test_keep_unsnapshoted() -> anyhow::Result<()> {
    let cases = vec![
        // last_deleted, last_applied, last_snapshoted, max_keep, want
        // empty test
        (None, None, None, 0, None),
        (None, None, None, 1, None),
        // nothing in snapshot
        (Some(log_id(1, 1)), Some(log_id(2, 2)), None, 0, None),
        (Some(log_id(1, 1)), Some(log_id(5, 5)), None, 0, None),
        // snapshot kept up
        (None, Some(log_id(5, 5)), Some(log_id(3, 4)), 0, Some(log_id(3, 4))),
        (
            Some(log_id(1, 1)),
            Some(log_id(5, 5)),
            Some(log_id(5, 5)),
            0,
            Some(log_id(5, 5)),
        ),
    ];

    for (purged, committed, snapshot, keep, want) in cases {
        let mut eng = eng();
        eng.config.keep_unsnapshoted_log = true;
        eng.config.max_applied_log_to_keep = keep;
        eng.config.purge_batch_size = 1;

        if let Some(last_purged) = purged {
            eng.state.log_ids.purge(&last_purged);
        }
        eng.state.committed = committed;
        eng.snapshot_last_log_id = snapshot;

        let got = eng.calc_purge_upto();

        assert_eq!(
            want, got,
            "case: purged: {:?}, committed: {:?}, snapshot: {:?}, keep: {}",
            purged, committed, snapshot, keep
        )
    }

    Ok(())
}
