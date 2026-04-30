//! Tests for [`Config`]'s clap-derived CLI parsing surface.
//!
//! The whole module is gated behind `feature = "clap"` in [`super`], so
//! individual tests do not need to repeat the gate.

use core::time::Duration;

use crate::Config;
use crate::SnapshotPolicy;

#[test]
fn test_build() -> anyhow::Result<()> {
    let config = Config::build(&[
        "foo",
        "--cluster-name=bar",
        "--election-timeout-min=10",
        "--election-timeout-max=20",
        "--heartbeat-interval=5",
        "--send-snapshot-timeout=199",
        "--install-snapshot-timeout=200",
        "--max-payload-entries=201",
        "--snapshot-policy=since_last:202",
        "--replication-lag-threshold=203",
        "--snapshot-max-chunk-size=204",
        "--max-in-snapshot-log-to-keep=205",
        "--purge-batch-size=207",
        "--api-channel-size=208",
        "--notification-channel-size=209",
    ])?;

    assert_eq!("bar", config.cluster_name);
    assert_eq!(10, config.election_timeout_min);
    assert_eq!(20, config.election_timeout_max);
    assert_eq!(5, config.heartbeat_interval);

    #[allow(deprecated)]
    {
        assert_eq!(199, config.send_snapshot_timeout);
    }
    assert_eq!(200, config.install_snapshot_timeout);
    assert_eq!(201, config.max_payload_entries);
    assert_eq!(SnapshotPolicy::LogsSinceLast(202), config.snapshot_policy);
    assert_eq!(203, config.replication_lag_threshold);
    assert_eq!(204, config.snapshot_max_chunk_size);
    assert_eq!(205, config.max_in_snapshot_log_to_keep);
    assert_eq!(207, config.purge_batch_size);
    assert_eq!(Some(208), config.api_channel_size);
    assert_eq!(Some(209), config.notification_channel_size);

    // Test config methods
    #[allow(deprecated)]
    {
        let mut c = config;
        assert_eq!(Duration::from_millis(199), c.send_snapshot_timeout());
        assert_eq!(Duration::from_millis(200), c.install_snapshot_timeout());

        c.send_snapshot_timeout = 0;
        assert_eq!(
            Duration::from_millis(200),
            c.send_snapshot_timeout(),
            "by default send_snapshot_timeout is install_snapshot_timeout"
        );
    }
    Ok(())
}

#[test]
fn test_config_snapshot_policy() -> anyhow::Result<()> {
    let config = Config::build(&["foo", "--snapshot-policy=never"])?;
    assert_eq!(SnapshotPolicy::Never, config.snapshot_policy);

    let config = Config::build(&["foo", "--snapshot-policy=since_last:3"])?;
    assert_eq!(SnapshotPolicy::LogsSinceLast(3), config.snapshot_policy);

    let res = Config::build(&["foo", "--snapshot-policy=bar:3"]);
    assert!(res.is_err());

    Ok(())
}

#[test]
fn test_config_enable_tick() -> anyhow::Result<()> {
    let config = Config::build(&["foo", "--enable-tick=false"])?;
    assert_eq!(false, config.enable_tick);

    let config = Config::build(&["foo", "--enable-tick=true"])?;
    assert_eq!(true, config.enable_tick);

    let config = Config::build(&["foo", "--enable-tick"])?;
    assert_eq!(true, config.enable_tick);

    let config = Config::build(&["foo"])?;
    assert_eq!(true, config.enable_tick);

    Ok(())
}

#[test]
fn test_config_enable_heartbeat() -> anyhow::Result<()> {
    let config = Config::build(&["foo", "--enable-heartbeat=false"])?;
    assert_eq!(false, config.enable_heartbeat);

    let config = Config::build(&["foo", "--enable-heartbeat=true"])?;
    assert_eq!(true, config.enable_heartbeat);

    let config = Config::build(&["foo", "--enable-heartbeat"])?;
    assert_eq!(true, config.enable_heartbeat);

    let config = Config::build(&["foo"])?;
    assert_eq!(true, config.enable_heartbeat);

    Ok(())
}

#[test]
fn test_config_enable_elect() -> anyhow::Result<()> {
    let config = Config::build(&["foo", "--enable-elect=false"])?;
    assert_eq!(false, config.enable_elect);

    let config = Config::build(&["foo", "--enable-elect=true"])?;
    assert_eq!(true, config.enable_elect);

    let config = Config::build(&["foo", "--enable-elect"])?;
    assert_eq!(true, config.enable_elect);

    let config = Config::build(&["foo"])?;
    assert_eq!(true, config.enable_elect);

    Ok(())
}

#[test]
fn test_config_allow_log_reversion() -> anyhow::Result<()> {
    let config = Config::build(&["foo", "--allow-log-reversion=false"])?;
    assert_eq!(Some(false), config.allow_log_reversion);

    let config = Config::build(&["foo", "--allow-log-reversion=true"])?;
    assert_eq!(Some(true), config.allow_log_reversion);

    let config = Config::build(&["foo", "--allow-log-reversion"])?;
    assert_eq!(Some(true), config.allow_log_reversion);

    let mut config = Config::build(&["foo"])?;
    assert_eq!(None, config.allow_log_reversion);

    // test allow_log_reversion method

    config.allow_log_reversion = None;
    assert_eq!(false, config.get_allow_log_reversion());

    config.allow_log_reversion = Some(true);
    assert_eq!(true, config.get_allow_log_reversion());

    config.allow_log_reversion = Some(false);
    assert_eq!(false, config.get_allow_log_reversion());

    Ok(())
}

#[test]
fn test_config_api_channel_size() -> anyhow::Result<()> {
    // Test default value
    let config = Config::build(&["foo"])?;
    assert_eq!(Some(65536), config.api_channel_size);
    assert_eq!(65536, config.api_channel_size());

    // Test custom value
    let config = Config::build(&["foo", "--api-channel-size=10000"])?;
    assert_eq!(Some(10000), config.api_channel_size);
    assert_eq!(10000, config.api_channel_size());

    // Test api_channel_size() method with None
    let mut config = Config::build(&["foo"])?;
    config.api_channel_size = None;
    assert_eq!(65536, config.api_channel_size());

    // Test api_channel_size() method with Some
    config.api_channel_size = Some(50000);
    assert_eq!(50000, config.api_channel_size());

    Ok(())
}

#[test]
fn test_config_notification_channel_size() -> anyhow::Result<()> {
    // Test default value
    let config = Config::build(&["foo"])?;
    assert_eq!(Some(65536), config.notification_channel_size);
    assert_eq!(65536, config.notification_channel_size());

    // Test custom value
    let config = Config::build(&["foo", "--notification-channel-size=2048"])?;
    assert_eq!(Some(2048), config.notification_channel_size);
    assert_eq!(2048, config.notification_channel_size());

    // Test notification_channel_size() method with None
    let mut config = Config::build(&["foo"])?;
    config.notification_channel_size = None;
    assert_eq!(65536, config.notification_channel_size());

    // Test notification_channel_size() method with Some
    config.notification_channel_size = Some(512);
    assert_eq!(512, config.notification_channel_size());

    Ok(())
}

#[test]
fn test_config_lifecycle_latency_capacity() -> anyhow::Result<()> {
    // Default: None (no clap default_value)
    let config = Config::build(&["foo"])?;
    assert_eq!(None, config.log_stage_capacity);
    assert_eq!(1024, config.log_stage_capacity());

    // Custom value via CLI
    let config = Config::build(&["foo", "--log-stage-capacity=512"])?;
    assert_eq!(Some(512), config.log_stage_capacity);
    assert_eq!(512, config.log_stage_capacity());

    // Method with None returns default
    let mut config = Config::build(&["foo"])?;
    config.log_stage_capacity = None;
    assert_eq!(1024, config.log_stage_capacity());

    // Method with Some returns the value
    config.log_stage_capacity = Some(2048);
    assert_eq!(2048, config.log_stage_capacity());

    Ok(())
}
