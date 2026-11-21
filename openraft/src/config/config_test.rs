use crate::Config;
use crate::SnapshotPolicy;
use crate::StepDownPolicy;
use crate::config::config::check_fits;
use crate::config::error::ConfigError;

#[test]
fn test_config_defaults() {
    let cfg = Config::default().validate().unwrap();

    assert_eq!(150, cfg.election_timeout_min);
    assert_eq!(300, cfg.election_timeout_max);

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
    assert_eq!(None, cfg.enable_relaxed_read);
    let relaxed_read = cfg.get_enable_relaxed_read();
    assert!(!relaxed_read);
}

#[test]
fn test_relaxed_read_configuration() {
    for (enable_relaxed_read, expected) in [(None, false), (Some(false), false), (Some(true), true)] {
        let config = Config {
            enable_relaxed_read,
            ..Default::default()
        };
        let enabled = config.get_enable_relaxed_read();
        assert_eq!(expected, enabled);
    }
}

#[cfg(feature = "serde")]
#[test]
fn test_relaxed_read_serde_default() -> anyhow::Result<()> {
    let config = Config {
        enable_relaxed_read: Some(true),
        ..Default::default()
    };
    let mut value = serde_json::to_value(config)?;
    let enabled: Config = serde_json::from_value(value.clone())?;
    assert_eq!(Some(true), enabled.enable_relaxed_read);
    value.as_object_mut().unwrap().remove("enable_relaxed_read");
    let default: Config = serde_json::from_value(value)?;
    assert_eq!(None, default.enable_relaxed_read);
    let enabled = default.get_enable_relaxed_read();
    assert!(!enabled);
    Ok(())
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
fn test_log_stage_capacity_configuration() {
    let config = Config {
        log_stage_capacity: Some(0),
        ..Default::default()
    };
    let result = config.validate();
    assert_eq!(ConfigError::LogStageCapacityIs0, result.unwrap_err());

    let config = Config {
        log_stage_capacity: Some(1),
        ..Default::default()
    };
    let config = config.validate().unwrap();
    assert_eq!(1, config.log_stage_capacity());

    let config = Config::default().validate().unwrap();
    assert_eq!(1024, config.log_stage_capacity());

    // On a 32-bit target `usize::MAX` is `u32::MAX`: 2^32 would truncate to 0, so it is rejected.
    let max_32 = u64::from(u32::MAX);
    let truncated = check_fits("log_stage_capacity", 1 << 32, max_32);
    let expected = ConfigError::TooLarge {
        field: "log_stage_capacity",
        value: 1 << 32,
        max: max_32,
    };
    assert_eq!(Err(expected), truncated);
}

/// `log_stage_capacity` must fit `usize`: on a 32-bit target, 2^32 is rejected instead of being
/// truncated to 0, which would reach `RangeMap::new(0)` when `RuntimeStats` is built.
#[cfg(target_pointer_width = "32")]
#[test]
fn test_log_stage_capacity_must_fit_usize() {
    let config = Config {
        log_stage_capacity: Some(1 << 32),
        ..Default::default()
    };
    let error = config.validate().map(|_| ()).unwrap_err();
    assert!(
        matches!(error, ConfigError::TooLarge {
            field: "log_stage_capacity",
            ..
        }),
        "{error:?}"
    );
}

#[test]
fn test_heartbeat_interval_must_be_positive() {
    for enable_heartbeat in [true, false] {
        let config = Config {
            heartbeat_interval: 0,
            enable_heartbeat,
            ..Default::default()
        };
        let result = config.validate().map(|_| ());
        assert_eq!(
            Err(ConfigError::HeartbeatIntervalIs0),
            result,
            "enable_heartbeat={enable_heartbeat}"
        );
    }

    let config = Config {
        heartbeat_interval: 1,
        election_timeout_min: 2,
        election_timeout_max: 3,
        ..Default::default()
    };
    let config = config.validate().unwrap();
    assert_eq!(1, config.heartbeat_interval);

    // A `heartbeat_min_interval` of 0 still turns heartbeat suppression off.
    let config = Config {
        heartbeat_min_interval: Some(0),
        ..Default::default()
    };
    let config = config.validate().unwrap();
    assert_eq!(0, config.heartbeat_min_interval());
}

#[test]
fn test_snapshot_logs_since_last_must_be_positive() {
    let config = Config {
        snapshot_policy: SnapshotPolicy::LogsSinceLast(0),
        ..Default::default()
    };
    assert_eq!(
        Err(ConfigError::SnapshotLogsSinceLastIs0),
        config.validate().map(|_| ())
    );

    for policy in [SnapshotPolicy::LogsSinceLast(1), SnapshotPolicy::Never] {
        let config = Config {
            snapshot_policy: policy.clone(),
            ..Default::default()
        };
        let config = config.validate().unwrap();
        assert_eq!(policy, config.snapshot_policy);
    }
}

#[cfg(feature = "serde")]
#[test]
fn test_snapshot_logs_since_last_zero_from_serde_fails_validation() -> anyhow::Result<()> {
    let mut value = serde_json::to_value(Config::default())?;
    value["snapshot_policy"] = serde_json::json!({ "LogsSinceLast": 0 });
    let config: Config = serde_json::from_value(value)?;
    assert_eq!(SnapshotPolicy::LogsSinceLast(0), config.snapshot_policy);
    assert_eq!(
        Err(ConfigError::SnapshotLogsSinceLastIs0),
        config.validate().map(|_| ())
    );
    Ok(())
}

#[test]
fn test_snapshot_max_chunk_size_must_form_a_byte_buffer() {
    let config = Config {
        snapshot_max_chunk_size: 0,
        ..Default::default()
    };
    assert_eq!(Err(ConfigError::SnapshotMaxChunkSizeIs0), config.validate().map(|_| ()));

    let max = u64::try_from(isize::MAX).unwrap();
    let config = Config {
        snapshot_max_chunk_size: max + 1,
        ..Default::default()
    };
    let expected = ConfigError::TooLarge {
        field: "snapshot_max_chunk_size",
        value: max + 1,
        max,
    };
    assert_eq!(Err(expected), config.validate().map(|_| ()));

    for size in [1, max, Config::default().snapshot_max_chunk_size] {
        let config = Config {
            snapshot_max_chunk_size: size,
            ..Default::default()
        };
        let config = config.validate().unwrap();
        assert_eq!(size, config.snapshot_max_chunk_size);
    }

    // On a 32-bit target `isize::MAX` is `i32::MAX`: a chunk size that would truncate to 0 there,
    // such as 2^32, is rejected.
    let max_32 = u64::try_from(i32::MAX).unwrap();
    let truncated = check_fits("snapshot_max_chunk_size", 1 << 32, max_32);
    assert!(matches!(truncated, Err(ConfigError::TooLarge { .. })), "{truncated:?}");
}

#[test]
fn test_channel_sizes_must_fit_usize() {
    // On a 32-bit target, `usize::MAX` is `u32::MAX`: a larger channel size must be rejected, not
    // truncated, possibly to 0.
    let max = u64::from(u32::MAX);
    assert_eq!(Ok(()), check_fits("api_channel_size", max, max));
    let too_large = check_fits("api_channel_size", max + 1, max);
    let expected = ConfigError::TooLarge {
        field: "api_channel_size",
        value: max + 1,
        max,
    };
    assert_eq!(Err(expected), too_large);

    // `Config::validate` checks each channel size against `usize::MAX`. On a 64-bit target every
    // `u64` fits; the runtime check in `Raft::new` rejects what its channels do not accept.
    #[cfg(target_pointer_width = "64")]
    {
        let config = Config {
            api_channel_size: Some(u64::MAX),
            ..Default::default()
        };
        let config = config.validate().unwrap();
        assert_eq!(usize::MAX, config.api_channel_size());
    }
}

#[test]
fn test_validate_channel_sizes() {
    let config = Config {
        api_channel_size: Some(2),
        notification_channel_size: Some(3),
        state_machine_channel_size: Some(4),
        ..Default::default()
    };
    assert_eq!(Ok(()), config.validate_channel_sizes(2..=4));

    // The first size outside the accepted sizes is reported, in field order.
    for (accepted, field, size) in [
        (3..=4, "api_channel_size", 2),
        (2..=2, "notification_channel_size", 3),
        (2..=3, "state_machine_channel_size", 4),
    ] {
        let result = config.validate_channel_sizes(accepted.clone());
        let expected = ConfigError::ChannelSizeOutOfRange { field, size, accepted };
        assert_eq!(Err(expected), result);
    }
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
    let config = config.validate().unwrap();
    assert_eq!(Some(600), config.quorum_loss_probe_interval);
    assert_eq!(300, config.election_timeout_max);

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

    assert_eq!(None, config.api_batch_capacity);
    assert_eq!(4096, config.api_batch_capacity());
    assert_eq!(None, config.api_batch_linger_ms);
    assert_eq!(0, config.api_batch_linger_ms());
    assert_eq!(None, config.backoff);
    assert_eq!("200ms", config.get_backoff());
    assert_eq!(StepDownPolicy::After(150), config.removed_leader_step_down);
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

    // Equal bounds leave an empty range to sample an election timeout from.
    let config = Config {
        election_timeout_min: 1000,
        election_timeout_max: 1000,
        ..Default::default()
    };
    let err = config.validate().unwrap_err();
    assert_eq!(err, ConfigError::ElectionTimeout { min: 1000, max: 1000 });

    // A heartbeat interval equal to the election minimum. Suppression is off, so that its own
    // check cannot reject the setting instead.
    let config = Config {
        election_timeout_min: 1000,
        election_timeout_max: 2000,
        heartbeat_interval: 1000,
        heartbeat_min_interval: None,
        ..Default::default()
    };
    let err = config.validate().unwrap_err();
    assert_eq!(err, ConfigError::ElectionTimeoutLTHeartBeat {
        election_timeout_min: 1000,
        heartbeat_interval: 1000
    });

    // One millisecond inside both bounds is valid.
    let config = Config {
        election_timeout_min: 1000,
        election_timeout_max: 1001,
        heartbeat_interval: 999,
        heartbeat_min_interval: None,
        ..Default::default()
    };
    let config = config.validate().unwrap();
    assert_eq!(1000, config.election_timeout_min);
    assert_eq!(1001, config.election_timeout_max);
    assert_eq!(999, config.heartbeat_interval);
}

#[test]
fn test_invalid_heartbeat_min_interval_produces_expected_error() {
    // The greatest accepted `heartbeat_min_interval` is
    // `election_timeout_min - heartbeat_interval - 1` == 499.

    let config = Config {
        election_timeout_min: 1000,
        election_timeout_max: 2000,
        heartbeat_interval: 500,
        heartbeat_min_interval: Some(500),
        ..Default::default()
    };

    let res = config.validate();
    let err = res.unwrap_err();
    assert_eq!(err, ConfigError::HeartbeatMinIntervalTooLarge {
        election_timeout_min: 1000,
        heartbeat_interval: 500,
        heartbeat_min_interval: 500,
    });

    let config = Config {
        election_timeout_min: 1000,
        election_timeout_max: 2000,
        heartbeat_interval: 500,
        heartbeat_min_interval: Some(499),
        ..Default::default()
    };
    let config = config.validate().unwrap();
    assert_eq!(Some(499), config.heartbeat_min_interval);
    assert_eq!(499, config.heartbeat_min_interval());
    assert_eq!(1000, config.election_timeout_min);
    assert_eq!(2000, config.election_timeout_max);
    assert_eq!(500, config.heartbeat_interval);
}

/// The heartbeat gap is computed in `u128`, so a huge `heartbeat_min_interval` is rejected with the
/// usual error instead of overflowing: a debug build panicked, and a release build wrapped the sum
/// to 49 and accepted it.
#[test]
fn test_heartbeat_min_interval_overflow_returns_an_error() {
    let config = Config {
        election_timeout_min: 150,
        election_timeout_max: 300,
        heartbeat_interval: 50,
        heartbeat_min_interval: Some(u64::MAX),
        ..Default::default()
    };

    let err = config.validate().unwrap_err();
    assert_eq!(err, ConfigError::HeartbeatMinIntervalTooLarge {
        election_timeout_min: 150,
        heartbeat_interval: 50,
        heartbeat_min_interval: u64::MAX,
    });
}

/// Suppression disabled leaves the heartbeat cadence unchanged, so `election_timeout_min` only has
/// to exceed `heartbeat_interval`.
#[test]
fn test_disabled_heartbeat_min_interval_only_needs_a_greater_election_timeout() {
    let config = Config {
        election_timeout_min: 101,
        election_timeout_max: 200,
        heartbeat_interval: 100,
        heartbeat_min_interval: None,
        ..Default::default()
    };
    let config = config.validate().unwrap();
    assert_eq!(None, config.heartbeat_min_interval);
    assert_eq!(0, config.heartbeat_min_interval());
    assert_eq!(101, config.election_timeout_min);
    assert_eq!(200, config.election_timeout_max);
    assert_eq!(100, config.heartbeat_interval);
}
