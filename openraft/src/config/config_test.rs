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
    assert_eq!(None, cfg.quorum_loss_grace);
    assert_eq!(None, cfg.quorum_loss_probe_interval);
}

#[test]
fn test_quorum_loss_configuration() {
    for probe_interval in [None, Some(0), Some(u64::MAX)] {
        assert!(
            Config {
                quorum_loss_probe_interval: probe_interval,
                ..Default::default()
            }
            .validate()
            .is_ok()
        );
    }

    assert_eq!(
        ConfigError::QuorumLossProbeIntervalRequired,
        Config {
            quorum_loss_grace: Some(0),
            ..Default::default()
        }
        .validate()
        .unwrap_err()
    );

    for grace in [0, 500] {
        for probe_interval in [0, 599, 600, 601] {
            let result = Config {
                quorum_loss_grace: Some(grace),
                quorum_loss_probe_interval: Some(probe_interval),
                ..Default::default()
            }
            .validate();
            if probe_interval > 600 {
                assert!(result.is_ok());
            } else {
                assert_eq!(
                    ConfigError::QuorumLossProbeIntervalTooSmall {
                        election_timeout_max: 300,
                        probe_interval,
                    },
                    result.unwrap_err()
                );
            }
        }
    }

    assert_eq!(
        ConfigError::QuorumLossProbeIntervalTooSmall {
            election_timeout_max: u64::MAX,
            probe_interval: u64::MAX,
        },
        Config {
            election_timeout_max: u64::MAX,
            quorum_loss_grace: Some(0),
            quorum_loss_probe_interval: Some(u64::MAX),
            ..Default::default()
        }
        .validate()
        .unwrap_err()
    );
}

#[cfg(feature = "serde")]
#[test]
fn test_quorum_loss_serde_default() -> anyhow::Result<()> {
    let mut value = serde_json::to_value(Config::default())?;
    let fields = value.as_object_mut().unwrap();
    fields.remove("quorum_loss_grace");
    fields.remove("quorum_loss_probe_interval");

    let cfg: Config = serde_json::from_value(value)?;
    assert_eq!(None, cfg.quorum_loss_grace);
    assert_eq!(None, cfg.quorum_loss_probe_interval);
    assert!(cfg.validate().is_ok());

    Ok(())
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
