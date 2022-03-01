use crate::core::is_matched_upto_date;
use crate::testing::DummyConfig;
use crate::Config;
use crate::LeaderId;
use crate::LogId;

#[test]
fn test_is_line_rate() -> anyhow::Result<()> {
    let m = Some(LogId::<DummyConfig>::new(LeaderId::new(1, 0), 10));

    let cfg = |n| Config {
        replication_lag_threshold: n,
        ..Default::default()
    };

    assert!(
        is_matched_upto_date::<DummyConfig>(&None, &None, &cfg(0)),
        "matched, threshold=0"
    );
    assert!(
        is_matched_upto_date::<DummyConfig>(
            &None,
            &Some(LogId {
                leader_id: LeaderId::new(2, 0),
                index: 0
            }),
            &cfg(1)
        ),
        "matched, threshold=1"
    );
    assert!(
        !is_matched_upto_date::<DummyConfig>(
            &None,
            &Some(LogId {
                leader_id: LeaderId::new(2, 0),
                index: 0
            }),
            &cfg(0)
        ),
        "not matched, threshold=1"
    );

    assert!(
        is_matched_upto_date::<DummyConfig>(&Some(LogId::new(LeaderId::new(0, 0), 0)), &None, &cfg(0)),
        "matched, threshold=0"
    );

    assert!(
        is_matched_upto_date::<DummyConfig>(&m, &Some(LogId::new(LeaderId::new(2, 0), 10)), &cfg(0)),
        "matched, threshold=0"
    );
    assert!(
        is_matched_upto_date::<DummyConfig>(&m, &Some(LogId::new(LeaderId::new(2, 0), 9)), &cfg(0)),
        "overflow, threshold=0"
    );
    assert!(
        !is_matched_upto_date(&m, &Some(LogId::new(LeaderId::new(2, 0), 11)), &cfg(0)),
        "not caught up, threshold=0"
    );
    assert!(
        is_matched_upto_date::<DummyConfig>(&m, &Some(LogId::new(LeaderId::new(2, 0), 11)), &cfg(1)),
        "caught up, threshold=1"
    );
    Ok(())
}
