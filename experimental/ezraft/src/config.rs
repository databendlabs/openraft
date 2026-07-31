//! Configuration for EzRaft
//!
//! EzRaft provides sensible defaults for all Raft timing parameters.
//! Users can optionally customize a few key parameters.

use std::io;
use std::time::Duration;

use openraft::Config;
use openraft::SnapshotPolicy;

/// Configuration for EzRaft
///
/// Provides sensible defaults for all Raft timing parameters.
/// Most users can use `EzConfig::default()` directly.
///
/// Election timeout is automatically calculated as 3-6x the heartbeat interval.
#[derive(Clone, Debug)]
pub struct EzConfig {
    /// Heartbeat interval
    ///
    /// Leader sends heartbeats to followers at this interval.
    /// Election timeout is derived from this value (3x to 6x).
    /// Default: 500ms
    pub heartbeat_interval: Duration,

    /// Number of new log entries between automatic snapshots
    ///
    /// After this many entries since the last snapshot, the framework builds a new one -
    /// [`crate::EzStorage::persist`] receives [`crate::Persist::Snapshot`] - and the entries it
    /// covers become eligible for deletion via [`crate::Persist::DeleteLogs`]. The distance kept
    /// before deletion is derived from this value.
    ///
    /// The default is low enough that a first demo session exercises the whole persist
    /// lifecycle, snapshot and log deletion included, instead of leaving those paths untested.
    /// Default: 500
    pub snapshot_interval: u64,
}

impl Default for EzConfig {
    fn default() -> Self {
        Self {
            heartbeat_interval: Duration::from_millis(500),
            snapshot_interval: 500,
        }
    }
}

impl EzConfig {
    /// Convert to openraft Config
    ///
    /// Validates and creates the internal Raft configuration.
    /// Election timeout is calculated as 3x to 6x the heartbeat interval.
    pub(crate) fn to_raft_config(&self) -> Result<Config, io::Error> {
        let heartbeat_ms = self.heartbeat_interval.as_millis() as u64;

        let config = Config {
            heartbeat_interval: heartbeat_ms,
            election_timeout_min: heartbeat_ms * 3,
            election_timeout_max: heartbeat_ms * 6,
            snapshot_policy: SnapshotPolicy::LogsSinceLast(self.snapshot_interval),
            // Keep openraft's default proportions: retain 1/5 of the interval for
            // lagging followers, and openraft requires the lag threshold to be at
            // least the snapshot interval or a snapshot may not fix the lag.
            max_in_snapshot_log_to_keep: self.snapshot_interval / 5,
            replication_lag_threshold: self.snapshot_interval,
            ..Default::default()
        };

        config.validate().map_err(io::Error::other)
    }
}
