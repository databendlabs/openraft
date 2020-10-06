//! Raft runtime configuration.

use rand::{thread_rng, Rng};
use serde::{Deserialize, Serialize};

use crate::error::ConfigError;

/// Default election timeout minimum, in milliseconds.
pub const DEFAULT_ELECTION_TIMEOUT_MIN: u64 = 150;
/// Default election timeout maximum, in milliseconds.
pub const DEFAULT_ELECTION_TIMEOUT_MAX: u64 = 300;
/// Default heartbeat interval.
pub const DEFAULT_HEARTBEAT_INTERVAL: u64 = 50;
/// Default threshold for when to trigger a snapshot.
pub const DEFAULT_LOGS_SINCE_LAST: u64 = 5000;
/// Default maximum number of entries per replication payload.
pub const DEFAULT_MAX_PAYLOAD_ENTRIES: u64 = 300;
/// Default replication lag threshold.
pub const DEFAULT_REPLICATION_LAG_THRESHOLD: u64 = 1000;
/// Default snapshot chunksize.
pub const DEFAULT_SNAPSHOT_CHUNKSIZE: u64 = 1024 * 1024 * 3;

/// Log compaction and snapshot policy.
///
/// This governs when periodic snapshots will be taken, and also governs the conditions which
/// would cause a leader to send an `InstallSnapshot` RPC to a follower based on replication lag.
///
/// Additional policies may become available in the future.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SnapshotPolicy {
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
/// The default values used by this type should generally work well for Raft clusters which will
/// be running with nodes in multiple datacenter availability zones with low latency between
/// zones. These values should typically be made configurable from the perspective of the
/// application which is being built on top of Raft.
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
/// What does all of this mean? Simply keep your election timeout settings high enough that the
/// performance of your network will not cause election timeouts, but don't keep it so high that
/// a real leader crash would cause prolonged downtime. See the Raft spec §5.6 for more details.
#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    /// The application specific name of this Raft cluster.
    ///
    /// This does not influence the Raft protocol in any way, but is useful for observability.
    pub cluster_name: String,
    /// The minimum election timeout in milliseconds.
    pub election_timeout_min: u64,
    /// The maximum election timeout in milliseconds.
    pub election_timeout_max: u64,
    /// The heartbeat interval in milliseconds at which leaders will send heartbeats to followers.
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
    /// When configuring this value, it is important to note that setting this value too low could
    /// cause sub-optimal performance. This will primarily impact the speed at which slow nodes,
    /// nodes which have been offline, or nodes which are new to the cluster, are brought
    /// up-to-speed. If this is too low, it will take longer for the nodes to be brought up to
    /// consistency with the rest of the cluster.
    pub max_payload_entries: u64,
    /// The distance behind in log replication a follower must fall before it is considered "lagging".
    ///
    /// This configuration parameter controls replication streams from the leader to followers in
    /// the cluster. Once a replication stream is considered lagging, it will stop buffering
    /// entries being replicated, and instead will fetch entries directly from the log until it is
    /// up-to-speed, at which time it will transition out of "lagging" state back into "line-rate" state.
    pub replication_lag_threshold: u64,
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
    pub fn build(cluster_name: String) -> ConfigBuilder {
        ConfigBuilder {
            cluster_name,
            election_timeout_min: None,
            election_timeout_max: None,
            heartbeat_interval: None,
            max_payload_entries: None,
            replication_lag_threshold: None,
            snapshot_policy: None,
            snapshot_max_chunk_size: None,
        }
    }

    /// Generate a new random election timeout within the configured min & max.
    pub fn new_rand_election_timeout(&self) -> u64 {
        thread_rng().gen_range(self.election_timeout_min, self.election_timeout_max)
    }
}

/// A configuration builder to ensure that runtime config is valid.
///
/// For election timeout config & heartbeat interval configuration, it is recommended that §5.6 of
/// the Raft spec is considered in order to set the appropriate values.
#[derive(Debug, Serialize, Deserialize)]
pub struct ConfigBuilder {
    /// The application specific name of this Raft cluster.
    pub cluster_name: String,
    /// The minimum election timeout, in milliseconds.
    pub election_timeout_min: Option<u64>,
    /// The maximum election timeout, in milliseconds.
    pub election_timeout_max: Option<u64>,
    /// The interval at which leaders will send heartbeats to followers to avoid election timeout.
    pub heartbeat_interval: Option<u64>,
    /// The maximum number of entries per payload allowed to be transmitted during replication.
    pub max_payload_entries: Option<u64>,
    /// The distance behind in log replication a follower must fall before it is considered "lagging".
    pub replication_lag_threshold: Option<u64>,
    /// The snapshot policy.
    pub snapshot_policy: Option<SnapshotPolicy>,
    /// The maximum snapshot chunk size.
    pub snapshot_max_chunk_size: Option<u64>,
}

impl ConfigBuilder {
    /// Set the desired value for `election_timeout_min`.
    pub fn election_timeout_min(mut self, val: u64) -> Self {
        self.election_timeout_min = Some(val);
        self
    }

    /// Set the desired value for `election_timeout_max`.
    pub fn election_timeout_max(mut self, val: u64) -> Self {
        self.election_timeout_max = Some(val);
        self
    }

    /// Set the desired value for `heartbeat_interval`.
    pub fn heartbeat_interval(mut self, val: u64) -> Self {
        self.heartbeat_interval = Some(val);
        self
    }

    /// Set the desired value for `max_payload_entries`.
    pub fn max_payload_entries(mut self, val: u64) -> Self {
        self.max_payload_entries = Some(val);
        self
    }

    /// Set the desired value for `replication_lag_threshold`.
    pub fn replication_lag_threshold(mut self, val: u64) -> Self {
        self.replication_lag_threshold = Some(val);
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
        // Roll a random election time out based on the configured min & max or their respective defaults.
        let election_timeout_min = self.election_timeout_min.unwrap_or(DEFAULT_ELECTION_TIMEOUT_MIN);
        let election_timeout_max = self.election_timeout_max.unwrap_or(DEFAULT_ELECTION_TIMEOUT_MAX);
        if election_timeout_min >= election_timeout_max {
            return Err(ConfigError::InvalidElectionTimeoutMinMax);
        }
        // Get other values or their defaults.
        let heartbeat_interval = self.heartbeat_interval.unwrap_or(DEFAULT_HEARTBEAT_INTERVAL);
        let max_payload_entries = self.max_payload_entries.unwrap_or(DEFAULT_MAX_PAYLOAD_ENTRIES);
        if max_payload_entries == 0 {
            return Err(ConfigError::MaxPayloadEntriesTooSmall);
        }
        let replication_lag_threshold = self.replication_lag_threshold.unwrap_or(DEFAULT_REPLICATION_LAG_THRESHOLD);
        let snapshot_policy = self.snapshot_policy.unwrap_or_else(SnapshotPolicy::default);
        let snapshot_max_chunk_size = self.snapshot_max_chunk_size.unwrap_or(DEFAULT_SNAPSHOT_CHUNKSIZE);
        Ok(Config {
            cluster_name: self.cluster_name,
            election_timeout_min,
            election_timeout_max,
            heartbeat_interval,
            max_payload_entries,
            replication_lag_threshold,
            snapshot_policy,
            snapshot_max_chunk_size,
        })
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// Unit Tests ////////////////////////////////////////////////////////////////////////////////////

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let cfg = Config::build("cluster0".into()).validate().unwrap();

        assert!(cfg.election_timeout_min >= DEFAULT_ELECTION_TIMEOUT_MIN as u64);
        assert!(cfg.election_timeout_max <= DEFAULT_ELECTION_TIMEOUT_MAX as u64);
        assert!(cfg.heartbeat_interval == DEFAULT_HEARTBEAT_INTERVAL as u64);
        assert!(cfg.max_payload_entries == DEFAULT_MAX_PAYLOAD_ENTRIES);
        assert!(cfg.replication_lag_threshold == DEFAULT_REPLICATION_LAG_THRESHOLD);
        assert!(cfg.snapshot_max_chunk_size == DEFAULT_SNAPSHOT_CHUNKSIZE);
        assert!(cfg.snapshot_policy == SnapshotPolicy::LogsSinceLast(DEFAULT_LOGS_SINCE_LAST));
    }

    #[test]
    fn test_config_with_specified_values() {
        let cfg = Config::build("cluster0".into())
            .election_timeout_max(200)
            .election_timeout_min(100)
            .heartbeat_interval(10)
            .max_payload_entries(100)
            .replication_lag_threshold(100)
            .snapshot_max_chunk_size(200)
            .snapshot_policy(SnapshotPolicy::LogsSinceLast(10000))
            .validate()
            .unwrap();

        assert!(cfg.election_timeout_min >= 100);
        assert!(cfg.election_timeout_max <= 200);
        assert!(cfg.heartbeat_interval == 10);
        assert!(cfg.max_payload_entries == 100);
        assert!(cfg.replication_lag_threshold == 100);
        assert!(cfg.snapshot_max_chunk_size == 200);
        assert!(cfg.snapshot_policy == SnapshotPolicy::LogsSinceLast(10000));
    }

    #[test]
    fn test_invalid_election_timeout_config_produces_expected_error() {
        let res = Config::build("cluster0".into())
            .election_timeout_min(1000)
            .election_timeout_max(700)
            .validate();
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert_eq!(err, ConfigError::InvalidElectionTimeoutMinMax);
    }
}
