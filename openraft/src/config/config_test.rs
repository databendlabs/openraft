use crate::config::error::ConfigError;
use crate::Config;
use crate::SnapshotPolicy;

#[test]
fn test_config_defaults() {
    let cfg = Config::default();

    assert!(cfg.election_timeout_min >= 150);
    assert!(cfg.election_timeout_max <= 300);

    assert_eq!(50, cfg.heartbeat_interval);
    assert_eq!(300, cfg.max_payload_entries);
    assert_eq!(1000, cfg.replication_lag_threshold);

    assert_eq!(3 * 1024 * 1024, cfg.snapshot_max_chunk_size);
    assert_eq!(SnapshotPolicy::LogsSinceLast(5000), cfg.snapshot_policy);
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
fn test_build() -> anyhow::Result<()> {
    let config = Config::build(&[
        "foo",
        "--cluster-name=bar",
        "--election-timeout-min=10",
        "--election-timeout-max=20",
        "--heartbeat-interval=5",
        "--install-snapshot-timeout=200",
        "--max-payload-entries=201",
        "--replication-lag-threshold=202",
        "--snapshot-policy=since_last:203",
        "--snapshot-max-chunk-size=204",
        "--max-in-snapshot-log-to-keep=205",
        "--purge-batch-size=207",
    ])?;

    assert_eq!("bar", config.cluster_name);
    assert_eq!(10, config.election_timeout_min);
    assert_eq!(20, config.election_timeout_max);
    assert_eq!(5, config.heartbeat_interval);
    assert_eq!(200, config.install_snapshot_timeout);
    assert_eq!(201, config.max_payload_entries);
    assert_eq!(202, config.replication_lag_threshold);
    assert_eq!(SnapshotPolicy::LogsSinceLast(203), config.snapshot_policy);
    assert_eq!(204, config.snapshot_max_chunk_size);
    assert_eq!(205, config.max_in_snapshot_log_to_keep);
    assert_eq!(207, config.purge_batch_size);

    Ok(())
}

#[test]
fn test_config_enable_tick() -> anyhow::Result<()> {
    let config = Config::build(&["foo", "--enable-tick=false"])?;
    assert!(!config.enable_tick);

    let config = Config::build(&["foo", "--enable-tick=true"])?;
    assert!(config.enable_tick);

    let config = Config::build(&["foo", "--enable-tick"])?;
    assert!(config.enable_tick);

    let config = Config::build(&["foo"])?;
    assert!(config.enable_tick);

    Ok(())
}

#[test]
fn test_config_enable_heartbeat() -> anyhow::Result<()> {
    let config = Config::build(&["foo", "--enable-heartbeat=false"])?;
    assert!(!config.enable_heartbeat);

    let config = Config::build(&["foo", "--enable-heartbeat=true"])?;
    assert!(config.enable_heartbeat);

    let config = Config::build(&["foo", "--enable-heartbeat"])?;
    assert!(config.enable_heartbeat);

    let config = Config::build(&["foo"])?;
    assert!(config.enable_heartbeat);

    Ok(())
}

#[test]
fn test_config_enable_elect() -> anyhow::Result<()> {
    let config = Config::build(&["foo", "--enable-elect=false"])?;
    assert!(!config.enable_elect);

    let config = Config::build(&["foo", "--enable-elect=true"])?;
    assert!(config.enable_elect);

    let config = Config::build(&["foo", "--enable-elect"])?;
    assert!(config.enable_elect);

    let config = Config::build(&["foo"])?;
    assert!(config.enable_elect);

    Ok(())
}
