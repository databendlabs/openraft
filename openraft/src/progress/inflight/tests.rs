use validit::Validate;

use crate::log_id_range::LogIdRange;
use crate::progress::Inflight;
use crate::CommittedLeaderId;
use crate::LogId;

fn log_id(index: u64) -> LogId<u64> {
    LogId {
        leader_id: CommittedLeaderId::new(1, 1),
        index,
    }
}

#[test]
fn test_inflight_create() -> anyhow::Result<()> {
    // Logs
    let l = Inflight::logs(Some(log_id(5)), Some(log_id(10)));
    assert_eq!(
        Inflight::Logs {
            id: 0,
            log_id_range: LogIdRange::new(Some(log_id(5)), Some(log_id(10)))
        },
        l
    );

    // Empty range
    let l = Inflight::logs(Some(log_id(11)), Some(log_id(10)));
    assert_eq!(Inflight::None, l);
    assert!(l.is_none());

    // Snapshot
    let l = Inflight::snapshot(Some(log_id(10)));
    assert_eq!(
        Inflight::Snapshot {
            id: 0,
            last_log_id: Some(log_id(10))
        },
        l
    );
    assert!(!l.is_none());

    Ok(())
}

#[test]
fn test_inflight_is_xxx() -> anyhow::Result<()> {
    let l = Inflight::<u64>::None;
    assert!(l.is_none());

    let l = Inflight::logs(Some(log_id(5)), Some(log_id(10)));
    assert!(l.is_sending_log());

    let l = Inflight::snapshot(Some(log_id(10)));
    assert!(l.is_sending_snapshot());

    Ok(())
}

#[test]
fn test_inflight_ack_with_invalid_request_id() -> anyhow::Result<()> {
    let mut f = Inflight::<u64>::None;
    let res = f.ack(1, Some(log_id(4)));
    assert!(res.is_err(), "Inflight::None can not ack");

    let mut f = Inflight::<u64>::logs(Some(log_id(5)), Some(log_id(10)));
    let res = f.ack(100, Some(log_id(4)));
    assert!(res.is_err(), "invalid request id for log");

    let mut f = Inflight::<u64>::snapshot(Some(log_id(5)));
    let res = f.ack(100, Some(log_id(4)));
    assert!(res.is_err(), "invalid request id for snapshot");
    Ok(())
}

#[test]
fn test_inflight_ack() -> anyhow::Result<()> {
    // Update matching when transmitting by logs
    {
        let mut f = Inflight::logs(Some(log_id(5)), Some(log_id(10)));

        f.ack(f.id(), Some(log_id(5)))?;
        assert_eq!(Inflight::logs(Some(log_id(5)), Some(log_id(10))), f);

        f.ack(f.id(), Some(log_id(6)))?;
        assert_eq!(Inflight::logs(Some(log_id(6)), Some(log_id(10))), f);

        f.ack(f.id(), Some(log_id(9)))?;
        assert_eq!(Inflight::logs(Some(log_id(9)), Some(log_id(10))), f);

        f.ack(f.id(), Some(log_id(10)))?;
        assert_eq!(Inflight::None, f);

        {
            let res = std::panic::catch_unwind(|| {
                let mut f = Inflight::logs(Some(log_id(5)), Some(log_id(10)));
                f.ack(f.id(), Some(log_id(4))).unwrap();
            });
            tracing::info!("res: {:?}", res);
            assert!(res.is_err(), "non-matching ack < prev_log_id");
        }

        {
            let res = std::panic::catch_unwind(|| {
                let mut f = Inflight::logs(Some(log_id(5)), Some(log_id(10)));
                f.ack(f.id(), Some(log_id(11))).unwrap();
            });
            tracing::info!("res: {:?}", res);
            assert!(res.is_err(), "non-matching ack > prev_log_id");
        }
    }

    // Update matching when transmitting by snapshot
    {
        {
            let mut f = Inflight::snapshot(Some(log_id(5)));
            f.ack(f.id(), Some(log_id(5)))?;
            assert_eq!(Inflight::None, f, "valid ack");
        }

        {
            let res = std::panic::catch_unwind(|| {
                let mut f = Inflight::snapshot(Some(log_id(5)));
                f.ack(f.id(), Some(log_id(4))).unwrap();
            });
            tracing::info!("res: {:?}", res);
            assert!(res.is_err(), "non-matching ack != snapshot.last_log_id");
        }
    }

    Ok(())
}

#[test]
fn test_inflight_ack_inherit_request_id() -> anyhow::Result<()> {
    let mut f = Inflight::logs(Some(log_id(5)), Some(log_id(10))).with_id(10);

    f.ack(f.id(), Some(log_id(5)))?;
    assert_eq!(Some(10), f.get_id());
    Ok(())
}

#[test]
fn test_inflight_conflict() -> anyhow::Result<()> {
    {
        let mut f = Inflight::logs(Some(log_id(5)), Some(log_id(10)));
        f.conflict(f.id(), 5)?;
        assert_eq!(Inflight::None, f, "valid conflict");
    }

    {
        let res = std::panic::catch_unwind(|| {
            let mut f = Inflight::logs(Some(log_id(5)), Some(log_id(10)));
            f.conflict(f.id(), 4).unwrap();
        });
        tracing::info!("res: {:?}", res);
        assert!(res.is_err(), "non-matching conflict < prev_log_id");
    }

    {
        let res = std::panic::catch_unwind(|| {
            let mut f = Inflight::logs(Some(log_id(5)), Some(log_id(10)));
            f.conflict(f.id(), 6).unwrap();
        });
        tracing::info!("res: {:?}", res);
        assert!(res.is_err(), "non-matching conflict > prev_log_id");
    }

    {
        let res = std::panic::catch_unwind(|| {
            let mut f = Inflight::snapshot(Some(log_id(5)));
            f.conflict(f.id(), 5).unwrap();
        });
        tracing::info!("res: {:?}", res);
        assert!(res.is_err(), "conflict is not expected by Inflight::Snapshot");
    }

    Ok(())
}
#[test]
fn test_inflight_conflict_invalid_request_id() -> anyhow::Result<()> {
    let mut f = Inflight::<u64>::None;
    let res = f.conflict(1, 5);
    assert!(res.is_err(), "conflict is not expected by Inflight::None");

    let mut f = Inflight::logs(Some(log_id(5)), Some(log_id(10)));
    let res = f.conflict(100, 5);
    assert!(res.is_err(), "conflict with invalid request id");
    Ok(())
}

#[test]
fn test_inflight_validate() -> anyhow::Result<()> {
    let r = Inflight::Logs {
        id: 0,
        log_id_range: LogIdRange::new(Some(log_id(5)), Some(log_id(4))),
    };
    let res = r.validate();
    assert!(res.is_err(), "prev(5) > last(4)");

    Ok(())
}
