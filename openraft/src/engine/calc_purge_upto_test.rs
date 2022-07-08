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

fn eng() -> Engine<u64> {
    let mut eng = Engine::<u64>::default();
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
    // last_purged_log_id, last_applied, max_keep, want
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
