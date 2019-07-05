//! The Raft configuration module.

use std::fs;

use failure::Fail;
use log::{error};
use rand::{thread_rng, Rng};

const DEFAULT_SNAPSHOT_CHUNKSIZE: u64 = 1024u64 * 1024u64 * 3;

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
    pub election_timeout_millis: u64,
    /// The heartbeat interval at which leaders will send heartbeats to followers.
    ///
    /// **NOTE WELL:** it is very important that this value be greater than the amount if time
    /// it will take on average for heartbeat frames to be sent between nodes. No data processing
    /// is performed for heartbeats, so the main item of concern here is network latency. This
    /// value is also used as the default timeout for sending heartbeats.
    pub heartbeat_interval: u64,
    /// The maximum number of entries per payload allowed to be transmitted during replication.
    ///
    /// When configuring this value, it is important to note that setting this value too low could
    /// cause sub-optimal performance. This will primarily impact the speed at which slow nodes,
    /// nodes which have been offline, or nodes which are new to the cluster, are brought
    /// up-to-speed. If this is too low, it will take longer for the nodes to be brought up to
    /// consistency with the rest of the cluster.
    pub max_payload_entries: u64,
    /// The directory where the log snapshots are to be kept for a Raft node.
    pub snapshot_dir: String,
    /// The snapshot policy to use for a Raft node.
    pub snapshot_policy: SnapshotPolicy,
    /// The maximum snapshot chunk size allowed when transmitting snapshots (in bytes).
    pub snapshot_max_chunk_size: u64,
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
            max_payload_entries: None,
            snapshot_dir,
            snapshot_policy: None,
            snapshot_max_chunk_size: None,
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
    /// The maximum number of entries per payload allowed to be transmitted during replication.
    ///
    /// This value defaults to 300.
    pub max_payload_entries: Option<u64>,
    /// The directory where the log snapshots are to be kept for a Raft node.
    snapshot_dir: String,
    /// The snapshot policy, defaults to `LogsSinceLast(5000)`.
    pub snapshot_policy: Option<SnapshotPolicy>,
    /// The maximum snapshot chunk size, defaults to 3MiB.
    pub snapshot_max_chunk_size: Option<u64>,
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

    /// Set the desired value for `max_payload_entries`.
    pub fn max_payload_entries(mut self, val: u64) -> Self {
        self.max_payload_entries = Some(val);
        self
    }

    /// Set the desired value for `snapshot_policy`.
    pub fn snapshot_policy(mut self, val: SnapshotPolicy) -> Self {
        self.snapshot_policy = Some(val);
        self
    }

    /// Set the desired value for `snapshot_max_chunk_size`.
    pub fn snapshot_max_chunk_size(mut self, val: u64) -> Self {
        self.snapshot_max_chunk_size = Some(val);
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

        // Get other values or their defaults.
        let heartbeat_interval = self.heartbeat_interval.unwrap_or(200) as u64;
        let max_payload_entries = self.max_payload_entries.unwrap_or(300);
        let snapshot_policy = self.snapshot_policy.unwrap_or_else(|| SnapshotPolicy::LogsSinceLast(5000));
        let snapshot_max_chunk_size = self.snapshot_max_chunk_size.unwrap_or(DEFAULT_SNAPSHOT_CHUNKSIZE);

        Ok(Config{
            election_timeout_millis,
            heartbeat_interval,
            max_payload_entries,
            snapshot_dir: self.snapshot_dir, snapshot_policy, snapshot_max_chunk_size,
        })
    }
}

/// A configuration error.
#[derive(Debug, Fail)]
pub enum ConfigError {
    /// The specified value for `snapshot_dir` does not exist on disk or could not be accessed.
    #[fail(display="The specified value for `snapshot_dir` does not exist on disk or could not be accessed.")]
    InvalidSnapshotDir,
}
