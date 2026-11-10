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
    assert_eq!(None, cfg.enable_unbounded_state_machine_buffer);
    let buffered = cfg.enable_unbounded_state_machine_buffer();
    assert!(!buffered);
    assert_eq!(None, cfg.broadcast_submitted_on_append);
    let broadcast_submitted = cfg.broadcast_submitted_on_append();
    assert!(!broadcast_submitted);
    assert_eq!(StepDownPolicy::After(150), cfg.removed_leader_step_down);
    assert_eq!(None, cfg.quorum_loss_probe_interval);
    assert_eq!(None, cfg.enable_local_lease_read);
    let local_lease_read = cfg.get_enable_local_lease_read();
    assert!(!local_lease_read);
}

#[test]
fn test_local_lease_read_configuration() {
    let cases = [(None, false), (Some(false), false), (Some(true), true)];
    for (enable_local_lease_read, expected) in cases {
        let config = Config {
            enable_local_lease_read,
            ..Default::default()
        };
        let enabled = config.get_enable_local_lease_read();
        assert_eq!(expected, enabled);
    }
}

#[cfg(feature = "serde")]
#[test]
fn test_local_lease_read_serde_default() -> anyhow::Result<()> {
    let config = Config {
        enable_local_lease_read: Some(true),
        ..Default::default()
    };
    let mut value = serde_json::to_value(config)?;
    let enabled: Config = serde_json::from_value(value.clone())?;
    assert_eq!(Some(true), enabled.enable_local_lease_read);
    let fields = value.as_object_mut().unwrap();
    fields.remove("enable_local_lease_read");
    let default: Config = serde_json::from_value(value)?;
    assert_eq!(None, default.enable_local_lease_read);
    let enabled = default.get_enable_local_lease_read();
    assert!(!enabled);
    Ok(())
}

#[cfg(feature = "serde")]
#[test]
fn test_state_machine_buffer_serde_default() -> anyhow::Result<()> {
    let config = Config {
        enable_unbounded_state_machine_buffer: Some(true),
        ..Default::default()
    };
    let mut value = serde_json::to_value(config)?;
    let enabled: Config = serde_json::from_value(value.clone())?;
    assert_eq!(Some(true), enabled.enable_unbounded_state_machine_buffer);
    let buffered = enabled.enable_unbounded_state_machine_buffer();
    assert!(buffered);
    let fields = value.as_object_mut().unwrap();
    fields.remove("enable_unbounded_state_machine_buffer");
    let default: Config = serde_json::from_value(value)?;
    assert_eq!(None, default.enable_unbounded_state_machine_buffer);
    let buffered = default.enable_unbounded_state_machine_buffer();
    assert!(!buffered);
    Ok(())
}

#[test]
fn test_broadcast_submitted_on_append() {
    let config = Config {
        broadcast_submitted_on_append: Some(true),
        ..Default::default()
    };
    let broadcast_submitted = config.broadcast_submitted_on_append();
    assert!(broadcast_submitted);
}

#[test]
fn test_quorum_loss_configuration() {
    let config = Config {
        quorum_loss_probe_interval: Some(599),
        ..Default::default()
    };
    let result = config.validate();
    let error = result.unwrap_err();
    assert_eq!(
        ConfigError::QuorumLossProbeIntervalTooSmall {
            election_timeout_max: 300,
            probe_interval: 599,
        },
        error
    );

    let config = Config {
        quorum_loss_probe_interval: Some(600),
        ..Default::default()
    };
    let result = config.validate();
    assert!(result.is_ok());

    let config = Config {
        election_timeout_max: u64::MAX,
        quorum_loss_probe_interval: Some(u64::MAX),
        ..Default::default()
    };
    let result = config.validate();
    let error = result.unwrap_err();
    assert_eq!(
        ConfigError::QuorumLossProbeIntervalTooSmall {
            election_timeout_max: u64::MAX,
            probe_interval: u64::MAX,
        },
        error
    );
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

#[cfg(feature = "serde")]
#[test]
fn test_reset_backoff_on_transfer_leader_serde_default() -> anyhow::Result<()> {
    let mut value = serde_json::to_value(Config::default())?;
    value.as_object_mut().unwrap().remove("reset_backoff_on_transfer_leader");

    let config: Config = serde_json::from_value(value)?;
    assert_eq!(None, config.reset_backoff_on_transfer_leader);
    let enabled = config.get_reset_backoff_on_transfer_leader();
    assert!(enabled);

    Ok(())
}

/// A config that Openraft 0.9 serialized deserializes and validates: every field added since then
/// takes its default, and the removed `send_snapshot_timeout` is ignored.
#[cfg(feature = "serde")]
#[test]
fn test_v09_config_deserializes() -> anyhow::Result<()> {
    // `serde_json::to_string(&Config::default())` with openraft 0.9.26, pretty-printed.
    let v09 = r#"{
        "cluster_name": "foo",
        "election_timeout_min": 150,
        "election_timeout_max": 300,
        "heartbeat_interval": 50,
        "install_snapshot_timeout": 200,
        "send_snapshot_timeout": 0,
        "max_payload_entries": 300,
        "replication_lag_threshold": 5000,
        "snapshot_policy": { "LogsSinceLast": 5000 },
        "snapshot_max_chunk_size": 3145728,
        "max_in_snapshot_log_to_keep": 1000,
        "purge_batch_size": 1,
        "enable_tick": true,
        "enable_heartbeat": true,
        "enable_elect": true
    }"#;

    let config: Config = serde_json::from_str(v09)?;

    let default = Config::default();
    assert_eq!(default.api_batch_capacity, config.api_batch_capacity);
    assert_eq!(default.api_batch_linger_ms, config.api_batch_linger_ms);
    assert_eq!(default.backoff, config.backoff);
    assert_eq!(default.removed_leader_step_down, config.removed_leader_step_down);
    assert_eq!(SnapshotPolicy::LogsSinceLast(5000), config.snapshot_policy);

    config.validate()?;

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
