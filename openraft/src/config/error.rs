/// Error variants related to configuration.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfigError {
    #[error("election timeout: min({min}) must be < max({max})")]
    ElectionTimeout { min: u64, max: u64 },

    #[error("max_payload_entries must be > 0")]
    MaxPayloadIs0,

    #[error("election_timeout_min({election_timeout_min}) must be > heartbeat_interval({heartbeat_interval})")]
    ElectionTimeoutLTHeartBeat {
        election_timeout_min: u64,
        heartbeat_interval: u64,
    },
}
