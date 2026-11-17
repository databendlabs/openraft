//! Opt-in application and storage costs for the in-process benchmark.

use std::sync::Arc;
use std::time::Duration;

use crate::profile_request::ProfileRequest;
use crate::store::ClientRequest;

/// Application workload and storage model; disabled for the empty control.
#[derive(Clone, Debug, clap::Args, serde::Serialize)]
#[openraft::macros::since(version = "0.10.1")]
pub struct Profile {
    /// Enable KV requests and the selected storage/transport costs.
    #[arg(long)]
    pub profile: bool,
    /// Bytes allocated independently for each application request.
    #[arg(long, default_value_t = 0)]
    pub payload_bytes: usize,
    /// Number of application keys.
    #[arg(long, default_value_t = 1024)]
    pub profile_keys: u64,
    /// Percentage of operations directed to key zero.
    #[arg(long, default_value_t = 0)]
    pub hot_key_percent: u64,
    /// Recent log entries considered cached; zero means every read misses.
    #[arg(long, default_value_t = 0)]
    pub cache_entries: u64,
    /// Delay paid once per cache-missing log read, in microseconds.
    #[arg(long, default_value_t = 0)]
    pub cache_miss_us: u64,
    /// Shared cache-miss log-read bandwidth in bytes/s; zero means unlimited.
    #[arg(long, default_value_t = 0)]
    pub log_read_bytes_per_second: u64,
    /// State-machine transaction bandwidth in bytes/s; zero means unlimited.
    #[arg(long, default_value_t = 0)]
    pub apply_bytes_per_second: u64,
    /// Actual deterministic mixing rounds per application byte (or ID for empty values).
    #[arg(long, default_value_t = 0)]
    pub cpu_rounds: u64,
    /// Prepare updates and perform I/O before publishing under a short write lock.
    #[arg(long)]
    pub concurrent_queries: bool,
    /// Serialize and deserialize all in-process RPC requests and responses.
    #[arg(long)]
    pub serialized_transport: bool,
    /// Shared bandwidth of each directed link in bytes/s; zero means unlimited.
    #[arg(long, default_value_t = 0)]
    pub link_bytes_per_second: u64,
    /// Maximum RPCs admitted by each directed link.
    #[arg(long, default_value_t = 64)]
    pub link_capacity: usize,
    /// Uniform added one-way link delay, from zero through this value.
    #[arg(long, default_value_t = 0)]
    pub network_jitter_us: u64,
    /// Additional read/apply delay on follower 1, in microseconds.
    #[arg(long, default_value_t = 0)]
    pub follower_delay_us: u64,
    /// Maximum logical IDs retained for duplicate detection on each state machine.
    #[arg(long, default_value_t = 65536)]
    pub duplicate_capacity: usize,
    /// Keep one in N correctness events (zero disables raw histories).
    #[arg(long, default_value_t = 128)]
    pub history_every: u64,
    /// Maximum retained correctness events.
    #[arg(long, default_value_t = 4096)]
    pub history_capacity: usize,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            profile: false,
            payload_bytes: 0,
            profile_keys: 1024,
            hot_key_percent: 0,
            cache_entries: 0,
            cache_miss_us: 0,
            log_read_bytes_per_second: 0,
            apply_bytes_per_second: 0,
            cpu_rounds: 0,
            concurrent_queries: false,
            serialized_transport: false,
            link_bytes_per_second: 0,
            link_capacity: 64,
            network_jitter_us: 0,
            follower_delay_us: 0,
            duplicate_capacity: 65536,
            history_every: 128,
            history_capacity: 4096,
        }
    }
}

impl Profile {
    /// Validate controls before constructing the cluster.
    #[openraft::macros::since(version = "0.10.1")]
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.profile_keys > 0, "profile-keys must be positive");
        anyhow::ensure!(self.hot_key_percent <= 100, "hot-key-percent must be at most 100");
        anyhow::ensure!(self.link_capacity > 0, "link-capacity must be positive");
        anyhow::ensure!(self.duplicate_capacity > 0, "duplicate-capacity must be positive");
        anyhow::ensure!(self.payload_bytes <= 16 * 1024 * 1024, "payload-bytes exceeds 16 MiB");
        let selected = self.payload_bytes > 0
            || self.hot_key_percent > 0
            || self.cache_entries > 0
            || self.cache_miss_us > 0
            || self.log_read_bytes_per_second > 0
            || self.apply_bytes_per_second > 0
            || self.cpu_rounds > 0
            || self.concurrent_queries
            || self.serialized_transport
            || self.follower_delay_us > 0
            || self.profile_keys != 1024
            || self.duplicate_capacity != 65536
            || self.history_every != 128
            || self.history_capacity != 4096;
        anyhow::ensure!(
            !selected || self.profile,
            "application/storage controls require --profile"
        );
        let wire = self.link_bytes_per_second > 0 || self.network_jitter_us > 0 || self.link_capacity != 64;
        anyhow::ensure!(
            !wire || self.serialized_transport,
            "link costs require --serialized-transport"
        );
        anyhow::ensure!(
            self.cache_miss_us < 390_000_000
                && self.follower_delay_us < 390_000_000
                && self.network_jitter_us < 390_000_000,
            "profile delay exceeds the benchmark deadline"
        );
        let work = (self.payload_bytes.max(1) as u128) * u128::from(self.cpu_rounds);
        anyhow::ensure!(
            work <= 100_000_000,
            "CPU work exceeds 100 million byte-rounds per entry"
        );
        Ok(())
    }

    /// Choose a reproducible key from an operation ID.
    #[openraft::macros::since(version = "0.10.1")]
    pub fn key(&self, operation: u64) -> u64 {
        let mixed = operation.wrapping_mul(0x9e3779b97f4a7c15);
        let mixed = mixed ^ (mixed >> 29);
        if mixed % 100 < self.hot_key_percent {
            return 0;
        }
        mixed % self.profile_keys
    }

    /// Allocate a request only when the application profile is enabled.
    #[openraft::macros::since(version = "0.10.1")]
    pub fn request(&self, operation: u64) -> ClientRequest {
        if !self.profile {
            return ClientRequest::default();
        }
        let key = self.key(operation);
        let value = vec![operation as u8; self.payload_bytes];
        let payload = ProfileRequest { operation, key, value };
        ClientRequest {
            payload: Some(Arc::new(payload)),
        }
    }

    /// Fixed follower asymmetry applied to log reads and state-machine transactions.
    #[openraft::macros::since(version = "0.10.1")]
    pub fn node_delay(&self, node: u64) -> Duration {
        if node != 1 {
            return Duration::ZERO;
        }
        Duration::from_micros(self.follower_delay_us)
    }
}

/// Duration required to transfer a byte count at the configured rate.
pub(crate) fn transfer_time(bytes: usize, rate: u64) -> Duration {
    if rate == 0 {
        return Duration::ZERO;
    }
    Duration::from_secs_f64(bytes as f64 / rate as f64)
}
