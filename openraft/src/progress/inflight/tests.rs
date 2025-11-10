use validit::Validate;

use crate::LogId;
use crate::engine::testing::UTConfig;
use crate::log_id_range::LogIdRange;
use crate::progress::Inflight;
use crate::type_config::alias::LeaderIdOf;
use crate::type_config::alias::LogIdOf;
use crate::vote::RaftLeaderIdExt;

fn log_id(index: u64) -> LogIdOf<UTConfig> {
    LogId {
        leader_id: LeaderIdOf::<UTConfig>::new_committed(1, 1),
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
    let l = Inflight::<UTConfig>::snapshot();
    assert_eq!(Inflight::Snapshot, l);
    assert!(!l.is_none());

    Ok(())
}

#[test]
fn test_inflight_is_xxx() -> anyhow::Result<()> {
    let l = Inflight::<UTConfig>::None;
    assert!(l.is_none());

    let l = Inflight::<UTConfig>::logs(Some(log_id(5)), Some(log_id(10)));
    assert!(l.is_sending_log());

    let l = Inflight::<UTConfig>::snapshot();
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
            let mut f = Inflight::<UTConfig>::snapshot();
            f.ack(Some(log_id(5)));
            assert_eq!(Inflight::<UTConfig>::None, f, "valid ack");
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
