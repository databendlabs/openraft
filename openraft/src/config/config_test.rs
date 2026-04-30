use crate::Config;
use crate::SnapshotPolicy;
use crate::config::error::ConfigError;

#[test]
fn test_config_defaults() {
    let cfg = Config::default();

    assert!(cfg.election_timeout_min >= 150);
    assert!(cfg.election_timeout_max <= 300);

    assert_eq!(50, cfg.heartbeat_interval);
    assert_eq!(300, cfg.max_payload_entries);
    assert_eq!(5000, cfg.replication_lag_threshold);

    assert_eq!(3 * 1024 * 1024, cfg.snapshot_max_chunk_size);
    assert_eq!(SnapshotPolicy::LogsSinceLast(5000), cfg.snapshot_policy);
    assert_eq!(Some(65536), cfg.api_channel_size);
    assert_eq!(Some(65536), cfg.notification_channel_size);
}

#[test]
fn test_invalid_election_timeout_config_produces_expected_error() {
    let config = Config {
        election_timeout_min: 1000,
        election_timeout_max: 700,
        ..Default::default()
    };

    let res = config.validate();
    let err = res.unwrap_err();
    assert_eq!(err, ConfigError::ElectionTimeout { min: 1000, max: 700 });

    let config = Config {
        election_timeout_min: 1000,
        election_timeout_max: 2000,
        heartbeat_interval: 1500,
        ..Default::default()
    };

    let res = config.validate();
    let err = res.unwrap_err();
    assert_eq!(err, ConfigError::ElectionTimeoutLTHeartBeat {
        election_timeout_min: 1000,
        heartbeat_interval: 1500
    });
}
