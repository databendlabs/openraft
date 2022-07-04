use crate::core::is_matched_upto_date;
use crate::types::v065::LogId;
use crate::Config;

#[test]
fn test_is_line_rate() -> anyhow::Result<()> {
    let m = LogId { term: 1, index: 10 };
    assert!(
        is_matched_upto_date(&m, &LogId { term: 2, index: 10 }, &Config {
            replication_lag_threshold: 0,
            ..Default::default()
        }),
        "matched, threshold=0"
    );
    assert!(
        is_matched_upto_date(&m, &LogId { term: 2, index: 9 }, &Config {
            replication_lag_threshold: 0,
            ..Default::default()
        }),
        "overflow, threshold=0"
    );
    assert!(
        !is_matched_upto_date(&m, &LogId { term: 2, index: 11 }, &Config {
            replication_lag_threshold: 0,
            ..Default::default()
        }),
        "not caught up, threshold=0"
    );
    assert!(
        is_matched_upto_date(&m, &LogId { term: 2, index: 11 }, &Config {
            replication_lag_threshold: 1,
            ..Default::default()
        }),
        "caught up, threshold=1"
    );
    Ok(())
}
