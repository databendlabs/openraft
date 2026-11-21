use std::ops::RangeInclusive;

use backoff_series::ParseError as BackoffParseError;
use openraft_macros::since;

use crate::impls::BoxedErrorSource;

/// Error variants related to configuration.
#[since]
#[since(version = "0.10.2", change = "added zero heartbeat interval error")]
#[since(version = "0.10.2", change = "added zero snapshot log threshold error")]
#[since(version = "0.10.2", change = "added zero snapshot chunk size error")]
#[since(
    version = "0.10.2",
    change = "added an error for a channel size the runtime does not accept"
)]
#[since(version = "0.10.2", change = "added an error for a value too large for the target")]
#[since(version = "0.10.2", change = "added zero log-stage capacity error")]
#[since(version = "0.10.0", change = "added quorum-loss inactivity validation error")]
#[derive(Debug, thiserror::Error)]
#[derive(PartialEq, Eq)]
pub enum ConfigError {
    /// Failed to parse configuration from command-line arguments.
    #[since(version = "0.10.0", change = "`source` from `AnyError` to `BoxedErrorSource`")]
    #[error("ParseError: {source} while parsing ({args:?})")]
    ParseError {
        /// The underlying parse error.
        source: BoxedErrorSource,
        /// The arguments that failed to parse.
        args: Vec<String>,
    },

    /// The min election timeout is not smaller than the max election timeout.
    #[error("election timeout: min({min}) must be < max({max})")]
    ElectionTimeout {
        /// Minimum election timeout value.
        min: u64,
        /// Maximum election timeout value.
        max: u64,
    },

    /// The `max_payload_entries` configuration must be greater than 0.
    #[error("max_payload_entries must be > 0")]
    MaxPayloadIs0,

    /// The threshold of [`SnapshotPolicy::LogsSinceLast`](crate::SnapshotPolicy::LogsSinceLast)
    /// must be greater than 0.
    #[since(version = "0.10.2")]
    #[error("snapshot_policy LogsSinceLast threshold must be > 0; use Never to disable automatic snapshots")]
    SnapshotLogsSinceLastIs0,

    /// The `snapshot_max_chunk_size` configuration must be greater than 0.
    #[since(version = "0.10.2")]
    #[error("snapshot_max_chunk_size must be > 0")]
    SnapshotMaxChunkSizeIs0,

    /// The `log_stage_capacity` configuration must be greater than 0.
    #[since(version = "0.10.2")]
    #[error("log_stage_capacity must be > 0")]
    LogStageCapacityIs0,

    /// A setting is larger than this target can hold, such as a size that does not fit in
    /// `usize` on a 32-bit target.
    #[since(version = "0.10.2")]
    #[error("{field}({value}) must be <= {max} on this target")]
    TooLarge {
        /// The name of the setting.
        field: &'static str,
        /// The configured value.
        value: u64,
        /// The largest value this target accepts for the setting.
        max: u64,
    },

    /// A channel size is outside the sizes that the runtime's channels accept, as given by
    /// [`Mpsc::buffer_size_range()`](crate::async_runtime::Mpsc::buffer_size_range).
    #[since(version = "0.10.2")]
    #[error("{field}({size}) must be in {accepted:?}, the sizes the runtime's channels accept")]
    ChannelSizeOutOfRange {
        /// The name of the channel size setting.
        field: &'static str,
        /// The channel size that the runtime would build.
        size: usize,
        /// The sizes that the runtime's channels accept.
        accepted: RangeInclusive<usize>,
    },

    /// Election timeout must be greater than heartbeat interval.
    #[error("election_timeout_min({election_timeout_min}) must be > heartbeat_interval({heartbeat_interval})")]
    ElectionTimeoutLTHeartBeat {
        /// Minimum election timeout value.
        election_timeout_min: u64,
        /// Heartbeat interval value.
        heartbeat_interval: u64,
    },

    /// The `heartbeat_interval` configuration must be greater than 0.
    #[since(version = "0.10.2")]
    #[error("heartbeat_interval must be > 0; use enable_heartbeat to turn heartbeats off")]
    HeartbeatIntervalIs0,

    /// Heartbeat suppression must not delay a heartbeat past a follower's election timeout.
    #[since(version = "0.10.1", change = "the bound no longer adds a tick interval")]
    #[since(version = "0.10.0")]
    #[error(
        "heartbeat_interval({heartbeat_interval}) + heartbeat_min_interval({heartbeat_min_interval}) must be < election_timeout_min({election_timeout_min})"
    )]
    HeartbeatMinIntervalTooLarge {
        /// Minimum election timeout value.
        election_timeout_min: u64,
        /// Heartbeat interval value.
        heartbeat_interval: u64,
        /// Minimum interval between two heartbeats to the same follower.
        heartbeat_min_interval: u64,
    },

    /// The probe interval must leave time for follower lease expiry and an election.
    #[since(version = "0.10.0", change = "added quorum-loss inactivity validation")]
    #[error(
        "quorum_loss_probe_interval({probe_interval}) must be >= leader_lease({election_timeout_max}) + election_timeout_max({election_timeout_max})"
    )]
    QuorumLossProbeIntervalTooSmall {
        /// Maximum election timeout in milliseconds.
        election_timeout_max: u64,
        /// Configured heartbeat phase interval in milliseconds.
        probe_interval: u64,
    },

    /// Invalid snapshot policy string format.
    #[error("snapshot policy string is invalid: '{invalid:?}' expect: '{syntax}'")]
    InvalidSnapshotPolicy {
        /// The invalid policy string provided.
        invalid: String,
        /// The expected syntax format.
        syntax: String,
    },

    /// Failed to parse a number from string.
    #[error("{reason} when parsing {invalid:?}")]
    InvalidNumber {
        /// The invalid number string.
        invalid: String,
        /// The reason for the parse failure.
        reason: String,
    },

    /// Invalid backoff policy string.
    ///
    /// See [`backoff_series::BackoffSeries::parse`] for the expected format.
    #[since(version = "0.10.0")]
    #[error(transparent)]
    InvalidBackoff(#[from] BackoffParseError),
}
