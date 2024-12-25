use validit::Validate;

use crate::engine::testing::UTConfig;
use crate::log_id_range::LogIdRange;
use crate::progress::Inflight;
use crate::CommittedLeaderId;
use crate::LogId;

fn log_id(index: u64) -> LogId<UTConfig> {
    LogId {
        leader_id: CommittedLeaderId::new(1, 1),
        index,
    }
}

#[test]
fn test_inflight_create() -> anyhow::Result<()> {
    // Logs
    let l = Inflight::<UTConfig>::logs(Some(log_id(5)), Some(log_id(10)));
    assert_eq!(
        Inflight::Logs {
            log_id_range: LogIdRange::new(Some(log_id(5)), Some(log_id(10)))
        },
        l
    );

    // Empty range
    let l = Inflight::<UTConfig>::logs(Some(log_id(11)), Some(log_id(10)));
    assert_eq!(Inflight::None, l);
    assert!(l.is_none());

    // Snapshot
    let l = Inflight::<UTConfig>::snapshot(Some(log_id(10)));
    assert_eq!(
        Inflight::Snapshot {
            last_log_id: Some(log_id(10))
        },
        l
    );
    assert!(!l.is_none());

    Ok(())
}

#[test]
fn test_inflight_is_xxx() -> anyhow::Result<()> {
    let l = Inflight::<UTConfig>::None;
    assert!(l.is_none());

    let l = Inflight::<UTConfig>::logs(Some(log_id(5)), Some(log_id(10)));
    assert!(l.is_sending_log());

    let l = Inflight::<UTConfig>::snapshot(Some(log_id(10)));
    assert!(l.is_sending_snapshot());

    Ok(())
}

#[test]
fn test_inflight_ack() -> anyhow::Result<()> {
    // Update matching when transmitting by logs
    {
        let mut f = Inflight::<UTConfig>::logs(Some(log_id(5)), Some(log_id(10)));

        f.ack(Some(log_id(5)));
        assert_eq!(Inflight::<UTConfig>::logs(Some(log_id(5)), Some(log_id(10))), f);

        f.ack(Some(log_id(6)));
        assert_eq!(Inflight::<UTConfig>::logs(Some(log_id(6)), Some(log_id(10))), f);

        f.ack(Some(log_id(9)));
        assert_eq!(Inflight::<UTConfig>::logs(Some(log_id(9)), Some(log_id(10))), f);

        f.ack(Some(log_id(10)));
        assert_eq!(Inflight::<UTConfig>::None, f);

        {
            let res = std::panic::catch_unwind(|| {
                let mut f = Inflight::<UTConfig>::logs(Some(log_id(5)), Some(log_id(10)));
                f.ack(Some(log_id(4)));
            });
            tracing::info!("res: {:?}", res);
            assert!(res.is_err(), "non-matching ack < prev_log_id");
        }

        {
            let res = std::panic::catch_unwind(|| {
                let mut f = Inflight::<UTConfig>::logs(Some(log_id(5)), Some(log_id(10)));
                f.ack(Some(log_id(11)));
            });
            tracing::info!("res: {:?}", res);
            assert!(res.is_err(), "non-matching ack > prev_log_id");
        }
    }

    // Update matching when transmitting by snapshot
    {
        {
            let mut f = Inflight::<UTConfig>::snapshot(Some(log_id(5)));
            f.ack(Some(log_id(5)));
            assert_eq!(Inflight::<UTConfig>::None, f, "valid ack");
        }

        {
            let res = std::panic::catch_unwind(|| {
                let mut f = Inflight::<UTConfig>::snapshot(Some(log_id(5)));
                f.ack(Some(log_id(4)));
            });
            tracing::info!("res: {:?}", res);
            assert!(res.is_err(), "non-matching ack != snapshot.last_log_id");
        }
    }

    Ok(())
}

#[test]
fn test_inflight_conflict() -> anyhow::Result<()> {
    {
        let mut f = Inflight::<UTConfig>::logs(Some(log_id(5)), Some(log_id(10)));
        f.conflict(5);
        assert_eq!(Inflight::<UTConfig>::None, f, "valid conflict");
    }

    {
        let res = std::panic::catch_unwind(|| {
            let mut f = Inflight::<UTConfig>::logs(Some(log_id(5)), Some(log_id(10)));
            f.conflict(4);
        });
        tracing::info!("res: {:?}", res);
        assert!(res.is_err(), "non-matching conflict < prev_log_id");
    }

    {
        let res = std::panic::catch_unwind(|| {
            let mut f = Inflight::<UTConfig>::logs(Some(log_id(5)), Some(log_id(10)));
            f.conflict(6);
        });
        tracing::info!("res: {:?}", res);
        assert!(res.is_err(), "non-matching conflict > prev_log_id");
    }

    {
        let res = std::panic::catch_unwind(|| {
            let mut f = Inflight::<UTConfig>::snapshot(Some(log_id(5)));
            f.conflict(5);
        });
        tracing::info!("res: {:?}", res);
        assert!(res.is_err(), "conflict is not expected by Inflight::Snapshot");
    }

    Ok(())
}

#[test]
fn test_inflight_validate() -> anyhow::Result<()> {
    let r = Inflight::Logs {
        log_id_range: LogIdRange::<UTConfig>::new(Some(log_id(5)), Some(log_id(4))),
    };
    let res = r.validate();
    assert!(res.is_err(), "prev(5) > last(4)");

    Ok(())
}
