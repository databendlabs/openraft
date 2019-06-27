//! The Raft configuration module.

use std::fs;

use failure::Fail;
use log::{error};
use rand::{thread_rng, Rng};

/// Raft log snapshot policy.
///
/// This governs when periodic snapshots will be taken, and also governs the conditions which
/// would cause a leader to send an `InstallSnapshot` RPC to a follower based on replication lag.
///
/// Additional policies may become available in the future.
#[derive(Debug)]
pub enum SnapshotPolicy {
    /// Snaphots are disabled for this Raft cluster.
    Disabled,
    /// A snapshot will be generated once the log has grown the specified number of logs since
    /// the last snapshot.
    LogsSinceLast(u64),
}

/// The runtime configuration for a Raft node.
#[derive(Debug)]
pub struct Config {
    /// The election timeout used for a Raft node when it is a follower.
    ///
    /// This value is randomly generated based on default confguration or a given min & max.
    pub(crate) election_timeout_millis: u64,
    /// The heartbeat interval at which leaders will send heartbeats to followers.
    pub(crate) heartbeat_interval: u64,
    /// The directory where the log snapshots are to be kept for a Raft node.
    pub(crate) snapshot_dir: String,
    /// The snapshot policy to use for a Raft node.
    pub(crate) snapshot_policy: SnapshotPolicy,
}

impl Config {
    /// Start the builder process for a new `Config` instance. Call `validate` when done.
    ///
    /// The directory where the log snapshots are to be kept for a Raft node is required and must
    /// be specified to start the config builder process.
    pub fn build(snapshot_dir: String) -> ConfigBuilder {
        ConfigBuilder{
            election_timeout_min: None,
            election_timeout_max: None,
            heartbeat_interval: None,
            snapshot_dir,
            snapshot_policy: None,
        }
    }
}

/// A configuration builder to ensure that the Raft's runtime config is valid.
///
/// For election timeout config & heartbeat interval configuration, it is recommended that §5.6 of
/// the Raft spec is considered in order to set the appropriate values.
#[derive(Debug)]
pub struct ConfigBuilder {
    /// The minimum election timeout in milliseconds, defaults to 500 milliseconds.
    pub election_timeout_min: Option<u16>,
    /// The maximum election timeout in milliseconds, defaults to 1000 milliseconds.
    pub election_timeout_max: Option<u16>,
    /// The interval at which leaders will send heartbeats to followers to avoid election timeout.
    ///
    /// This value defaults to 200 milliseconds.
    pub heartbeat_interval: Option<u16>,
    /// The directory where the log snapshots are to be kept for a Raft node.
    snapshot_dir: String,
    /// The snapshot policy, defaults to `LogsSinceLast(5000)`.
    pub snapshot_policy: Option<SnapshotPolicy>,
}

impl ConfigBuilder {
    /// Set the desired value for `election_timeout_min`.
    pub fn election_timeout_min(mut self, val: u16) -> Self {
        self.election_timeout_min = Some(val);
        self
    }

    /// Set the desired value for `election_timeout_max`.
    pub fn election_timeout_max(mut self, val: u16) -> Self {
        self.election_timeout_max = Some(val);
        self
    }

    /// Set the desired value for `heartbeat_interval`.
    pub fn heartbeat_interval(mut self, val: u16) -> Self {
        self.heartbeat_interval = Some(val);
        self
    }

    /// Set the desired value for `snapshot_policy`.
    pub fn snapshot_policy(mut self, val: SnapshotPolicy) -> Self {
        self.snapshot_policy = Some(val);
        self
    }

    /// Validate the state of this builder and produce a new `Config` instance if valid.
    pub fn validate(self) -> Result<Config, ConfigError> {
        // Validate that `snapshot_dir` is a real location on disk, or attempt to create it.
        fs::create_dir_all(&self.snapshot_dir).map_err(|err| {
            error!("Error while checking configured value for `snapshot_dir`. {}", err);
            ConfigError::InvalidSnapshotDir
        })?;

        // Roll a random election time out based on the configured min & max or their respective defaults.
        let mut rng = thread_rng();
        let election_timeout: u16 = rng.gen_range(self.election_timeout_min.unwrap_or(500), self.election_timeout_max.unwrap_or(1000));
        let election_timeout_millis = election_timeout as u64;

        // Get the heartbat interval.
        let heartbeat_interval = self.heartbeat_interval.unwrap_or(200) as u64;

        // Get the snapshot policy or its default value.
        let snapshot_policy = self.snapshot_policy.unwrap_or_else(|| SnapshotPolicy::LogsSinceLast(5000));

        Ok(Config{election_timeout_millis, heartbeat_interval, snapshot_dir: self.snapshot_dir, snapshot_policy})
    }
}

/// A configuration error.
#[derive(Debug, Fail)]
pub enum ConfigError {
    /// The specified value for `snapshot_dir` does not exist on disk or could not be accessed.
    #[fail(display="The specified value for `snapshot_dir` does not exist on disk or could not be accessed.")]
    InvalidSnapshotDir,
}
