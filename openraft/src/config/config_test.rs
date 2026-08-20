use crate::Config;
use crate::SnapshotPolicy;
use crate::StepDownPolicy;
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
    assert_eq!(StepDownPolicy::After(150), cfg.removed_leader_step_down);
}

/// A config serialized before `removed_leader_step_down` existed deserializes to the default
/// policy.
#[cfg(feature = "serde")]
#[test]
fn test_removed_leader_step_down_serde_default() -> anyhow::Result<()> {
    let mut value = serde_json::to_value(Config::default())?;
    value.as_object_mut().unwrap().remove("removed_leader_step_down");

    let cfg: Config = serde_json::from_value(value)?;
    assert_eq!(StepDownPolicy::After(150), cfg.removed_leader_step_down);

    Ok(())
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

#[test]
fn test_invalid_heartbeat_min_interval_produces_expected_error() {
    // The tick interval is `heartbeat_interval * 13 / 64` == 101, so the greatest accepted
    // `heartbeat_min_interval` is `election_timeout_min - heartbeat_interval - 101 - 1` == 398.

    let config = Config {
        election_timeout_min: 1000,
        election_timeout_max: 2000,
        heartbeat_interval: 500,
        heartbeat_min_interval: Some(399),
        ..Default::default()
    };

    let res = config.validate();
    let err = res.unwrap_err();
    assert_eq!(err, ConfigError::HeartbeatMinIntervalTooLarge {
        election_timeout_min: 1000,
        heartbeat_interval: 500,
        heartbeat_min_interval: 399,
    });

    let config = Config {
        election_timeout_min: 1000,
        election_timeout_max: 2000,
        heartbeat_interval: 500,
        heartbeat_min_interval: Some(398),
        ..Default::default()
    };
    assert!(config.validate().is_ok());
}

/// Suppression disabled leaves the heartbeat cadence unchanged, so `election_timeout_min` only has
/// to exceed `heartbeat_interval`, not the tick interval.
#[test]
fn test_disabled_heartbeat_min_interval_ignores_tick_interval() {
    let config = Config {
        election_timeout_min: 101,
        election_timeout_max: 200,
        heartbeat_interval: 100,
        heartbeat_min_interval: None,
        ..Default::default()
    };
    assert!(config.validate().is_ok());
}
