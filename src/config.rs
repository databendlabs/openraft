//! The Raft configuration module.

use std::{
    fs,
    time::Duration,
};

use log::{error};
use rand::{thread_rng, Rng};

/// Default election timeout minimum.
pub const DEFAULT_ELECTION_TIMEOUT_MIN: u16 = 200;
/// Default election timeout maximum.
pub const DEFAULT_ELECTION_TIMEOUT_MAX: u16 = 300;
/// Default heartbeat interval.
pub const DEFAULT_HEARTBEAT_INTERVAL: u16 = 50;
/// Default threshold for when to trigger a snapshot.
pub const DEFAULT_LOGS_SINCE_LAST: u64 = 5000;
/// Default maximum number of entries per replication payload.
pub const DEFAULT_MAX_PAYLOAD_ENTRIES: u64 = 300;
/// Default metrics rate.
pub const DEFAULT_METRICS_RATE: Duration = Duration::from_millis(5000);
/// Default snapshot chunksize.
pub const DEFAULT_SNAPSHOT_CHUNKSIZE: u64 = 1024 * 1024 * 3;

/// Raft log snapshot policy.
///
/// This governs when periodic snapshots will be taken, and also governs the conditions which
/// would cause a leader to send an `InstallSnapshot` RPC to a follower based on replication lag.
///
/// Additional policies may become available in the future.
#[derive(Clone, Debug, PartialEq)]
pub enum SnapshotPolicy {
    /// Snaphots are disabled for this Raft cluster.
    Disabled,
    /// A snapshot will be generated once the log has grown the specified number of logs since
    /// the last snapshot.
    LogsSinceLast(u64),
}

impl Default for SnapshotPolicy {
    fn default() -> Self {
        SnapshotPolicy::LogsSinceLast(DEFAULT_LOGS_SINCE_LAST)
    }
}

/// The runtime configuration for a Raft node.
///
/// When building the Raft configuration for your application, remember this inequality from the
/// Raft spec: `broadcastTime ≪ electionTimeout ≪ MTBF`.
///
/// > In this inequality `broadcastTime` is the average time it takes a server to send RPCs in
/// > parallel to every server in the cluster and receive their responses; `electionTimeout` is the
/// > election timeout described in Section 5.2; and `MTBF` is the average time between failures for
/// > a single server. The broadcast time should be an order of magnitude less than the election
/// > timeout so that leaders can reliably send the heartbeat messages required to keep followers
/// > from starting elections; given the randomized approach used for election timeouts, this
/// > inequality also makes split votes unlikely. The election timeout should be a few orders of
/// > magnitude less than `MTBF` so that the system makes steady progress. When the leader crashes,
/// > the system will be unavailable for roughly the election timeout; we would like this to
/// > represent only a small fraction of overall time.
///
/// What does all of this mean simply? Keep your election timeout settings high enough that the
/// performance of your network will not cause election timeouts, but don't keep it so high that
/// a real leader crash would cause prolonged downtime. See the Raft spec §5.6 for more details.
#[derive(Debug)]
pub struct Config {
    /// The election timeout used for a Raft node when it is a follower.
    ///
    /// This value is randomly generated based on default confguration or a given min & max. The
    /// default value will be between 200-300 milliseconds.
    pub election_timeout_millis: u64,
    /// The heartbeat interval at which leaders will send heartbeats to followers.
    ///
    /// Defaults to 50 milliseconds.
    ///
    /// **NOTE WELL:** it is very important that this value be greater than the amount if time
    /// it will take on average for heartbeat frames to be sent between nodes. No data processing
    /// is performed for heartbeats, so the main item of concern here is network latency. This
    /// value is also used as the default timeout for sending heartbeats.
    pub heartbeat_interval: u64,
    /// The maximum number of entries per payload allowed to be transmitted during replication.
    ///
    /// Defaults to 300.
    ///
    /// When configuring this value, it is important to note that setting this value too low could
    /// cause sub-optimal performance. This will primarily impact the speed at which slow nodes,
    /// nodes which have been offline, or nodes which are new to the cluster, are brought
    /// up-to-speed. If this is too low, it will take longer for the nodes to be brought up to
    /// consistency with the rest of the cluster.
    pub max_payload_entries: u64,
    /// The rate at which metrics will be pumped out from the Raft node.
    ///
    /// Defaults to 5 seconds.
    pub metrics_rate: Duration,
    /// The directory where the log snapshots are to be kept for a Raft node.
    pub snapshot_dir: String,
    /// The snapshot policy to use for a Raft node.
    pub snapshot_policy: SnapshotPolicy,
    /// The maximum snapshot chunk size allowed when transmitting snapshots (in bytes).
    ///
    /// Defaults to 3Mib.
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
            metrics_rate: None,
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
    /// The minimum election timeout in milliseconds.
    pub election_timeout_min: Option<u16>,
    /// The maximum election timeout in milliseconds.
    pub election_timeout_max: Option<u16>,
    /// The interval at which leaders will send heartbeats to followers to avoid election timeout.
    pub heartbeat_interval: Option<u16>,
    /// The maximum number of entries per payload allowed to be transmitted during replication.
    pub max_payload_entries: Option<u64>,
    /// The rate at which metrics will be pumped out from the Raft node.
    pub metrics_rate: Option<Duration>,
    /// The directory where the log snapshots are to be kept for a Raft node.
    snapshot_dir: String,
    /// The snapshot policy.
    pub snapshot_policy: Option<SnapshotPolicy>,
    /// The maximum snapshot chunk size.
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

    /// Set the desired value for `metrics_rate`.
    pub fn metrics_rate(mut self, val: Duration) -> Self {
        self.metrics_rate = Some(val);
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
        let election_min = self.election_timeout_min.unwrap_or(DEFAULT_ELECTION_TIMEOUT_MIN);
        let election_max = self.election_timeout_max.unwrap_or(DEFAULT_ELECTION_TIMEOUT_MAX);
        if election_min >= election_max {
            return Err(ConfigError::InvalidElectionTimeoutMinMax);
        }
        let mut rng = thread_rng();
        let election_timeout: u16 = rng.gen_range(election_min, election_max);
        let election_timeout_millis = election_timeout as u64;

        // Get other values or their defaults.
        let heartbeat_interval = self.heartbeat_interval.unwrap_or(DEFAULT_HEARTBEAT_INTERVAL) as u64;
        let max_payload_entries = self.max_payload_entries.unwrap_or(DEFAULT_MAX_PAYLOAD_ENTRIES);
        let metrics_rate = self.metrics_rate.unwrap_or(DEFAULT_METRICS_RATE);
        let snapshot_policy = self.snapshot_policy.unwrap_or_else(|| SnapshotPolicy::default());
        let snapshot_max_chunk_size = self.snapshot_max_chunk_size.unwrap_or(DEFAULT_SNAPSHOT_CHUNKSIZE);

        Ok(Config{
            election_timeout_millis,
            heartbeat_interval,
            max_payload_entries,
            metrics_rate,
            snapshot_dir: self.snapshot_dir, snapshot_policy, snapshot_max_chunk_size,
        })
    }
}

/// A configuration error.
#[derive(Debug, PartialEq)]
pub enum ConfigError {
    /// The specified value for `snapshot_dir` does not exist on disk or could not be accessed.
    InvalidSnapshotDir,
    /// The given values for election timeout min & max are invalid. Max must be greater than min.
    InvalidElectionTimeoutMinMax,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::InvalidSnapshotDir => write!(f, "The specified value for `snapshot_dir` does not exist on disk or could not be accessed."),
            ConfigError::InvalidElectionTimeoutMinMax => write!(f, "The given values for election timeout min & max are invalid. Max must be greater than min."),
        }
    }
}

impl std::error::Error for ConfigError {}

//////////////////////////////////////////////////////////////////////////////////////////////////
// Unit Tests ////////////////////////////////////////////////////////////////////////////////////

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir_in;

    #[test]
    fn test_config_defaults() {
        let dir = tempdir_in("/tmp").unwrap();
        let dirstring = dir.path().to_string_lossy().to_string();
        let cfg = Config::build(dirstring.clone()).validate().unwrap();

        assert!(cfg.election_timeout_millis >= DEFAULT_ELECTION_TIMEOUT_MIN as u64);
        assert!(cfg.election_timeout_millis <= DEFAULT_ELECTION_TIMEOUT_MAX as u64);
        assert!(cfg.heartbeat_interval == DEFAULT_HEARTBEAT_INTERVAL as u64);
        assert!(cfg.max_payload_entries == DEFAULT_MAX_PAYLOAD_ENTRIES);
        assert!(cfg.metrics_rate == DEFAULT_METRICS_RATE);
        assert!(cfg.snapshot_dir == dirstring);
        assert!(cfg.snapshot_max_chunk_size == DEFAULT_SNAPSHOT_CHUNKSIZE);
        assert!(cfg.snapshot_policy == SnapshotPolicy::LogsSinceLast(DEFAULT_LOGS_SINCE_LAST));
    }

    #[test]
    fn test_config_with_specified_values() {
        let dir = tempdir_in("/tmp").unwrap();
        let dirstring = dir.path().to_string_lossy().to_string();
        let cfg = Config::build(dirstring.clone())
            .election_timeout_max(200)
            .election_timeout_min(100)
            .heartbeat_interval(10)
            .max_payload_entries(100)
            .metrics_rate(Duration::from_millis(20000))
            .snapshot_max_chunk_size(200)
            .snapshot_policy(SnapshotPolicy::Disabled)
            .validate().unwrap();

        assert!(cfg.election_timeout_millis >= 100);
        assert!(cfg.election_timeout_millis <= 200);
        assert!(cfg.heartbeat_interval == 10);
        assert!(cfg.max_payload_entries == 100);
        assert!(cfg.max_payload_entries == 100);
        assert!(cfg.metrics_rate == Duration::from_millis(20000));
        assert!(cfg.snapshot_dir == dirstring);
        assert!(cfg.snapshot_max_chunk_size == 200);
        assert!(cfg.snapshot_policy == SnapshotPolicy::Disabled);
    }

    #[test]
    fn test_invalid_path_returns_expected_error() {
        let res = Config::build("/dev/someinvalidpath/definitely/doesn't/exist".to_string()).validate();
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert_eq!(err, ConfigError::InvalidSnapshotDir);
    }

    #[test]
    fn test_invalid_election_timeout_config_produces_expected_error() {
        let dir = tempdir_in("/tmp").unwrap();
        let dirstring = dir.path().to_string_lossy().to_string();
        let res = Config::build(dirstring.clone())
            .election_timeout_min(1000).election_timeout_max(700).validate();
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert_eq!(err, ConfigError::InvalidElectionTimeoutMinMax);
    }
}
