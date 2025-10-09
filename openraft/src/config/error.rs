use anyerror::AnyError;

/// Error variants related to configuration.
#[derive(Debug, thiserror::Error)]
#[derive(PartialEq, Eq)]
pub enum ConfigError {
    /// Failed to parse configuration from command-line arguments.
    #[error("ParseError: {source} while parsing ({args:?})")]
    ParseError {
        /// The underlying parse error.
        source: AnyError,
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

    /// Election timeout must be greater than heartbeat interval.
    #[error("election_timeout_min({election_timeout_min}) must be > heartbeat_interval({heartbeat_interval})")]
    ElectionTimeoutLTHeartBeat {
        /// Minimum election timeout value.
        election_timeout_min: u64,
        /// Heartbeat interval value.
        heartbeat_interval: u64,
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
}
