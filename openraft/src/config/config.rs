//! Raft runtime configuration.

use std::ops::Deref;
use std::str::FromStr;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use anyerror::AnyError;
use clap::Parser;
use rand::Rng;

use crate::config::error::ConfigError;
use crate::raft_state::LogStateReader;
use crate::AsyncRuntime;
use crate::LogIdOptionExt;
use crate::RaftTypeConfig;

/// Log compaction and snapshot policy.
///
/// This governs when periodic snapshots will be taken, and also governs the conditions which
/// would cause a leader to send an `InstallSnapshot` RPC to a follower based on replication lag.
///
/// Additional policies may become available in the future.
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum SnapshotPolicy {
    /// A snapshot will be generated once the log has grown the specified number of logs since
    /// the last snapshot.
    LogsSinceLast(u64),

    /// Openraft will never trigger a snapshot building.
    /// With this option, the application calls
    /// [`Raft::trigger().snapshot()`](`crate::raft::trigger::Trigger::snapshot`) to manually
    /// trigger a snapshot.
    Never,
}

impl SnapshotPolicy {
    pub(crate) fn should_snapshot<C>(&self, state: &impl Deref<Target = impl LogStateReader<C>>) -> bool
    where C: RaftTypeConfig {
        match self {
            SnapshotPolicy::LogsSinceLast(threshold) => {
                state.committed().next_index() >= state.snapshot_last_log_id().next_index() + threshold
            }
            SnapshotPolicy::Never => false,
        }
    }
}

/// Parse number with unit such as 5.3 KB
fn parse_bytes_with_unit(src: &str) -> Result<u64, ConfigError> {
    let res = byte_unit::Byte::from_str(src).map_err(|e| ConfigError::InvalidNumber {
        invalid: src.to_string(),
        reason: e.to_string(),
    })?;

    Ok(res.as_u64())
}

fn parse_snapshot_policy(src: &str) -> Result<SnapshotPolicy, ConfigError> {
    if src == "never" {
        return Ok(SnapshotPolicy::Never);
    }

    let elts = src.split(':').collect::<Vec<_>>();
    if elts.len() != 2 {
        return Err(ConfigError::InvalidSnapshotPolicy {
            syntax: "never|since_last:<num>".to_string(),
            invalid: src.to_string(),
        });
    }

    if elts[0] != "since_last" {
        return Err(ConfigError::InvalidSnapshotPolicy {
            syntax: "never|since_last:<num>".to_string(),
            invalid: src.to_string(),
        });
    }

    let n_logs = elts[1].parse::<u64>().map_err(|e| ConfigError::InvalidNumber {
        invalid: src.to_string(),
        reason: e.to_string(),
    })?;
    Ok(SnapshotPolicy::LogsSinceLast(n_logs))
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
#[derive(Clone, Debug, Parser)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct Config {
    /// The application specific name of this Raft cluster
    #[clap(long, default_value = "foo")]
    pub cluster_name: String,

    /// The minimum election timeout in milliseconds
    #[clap(long, default_value = "150")]
    pub election_timeout_min: u64,

    /// The maximum election timeout in milliseconds
    #[clap(long, default_value = "300")]
    pub election_timeout_max: u64,

    /// The heartbeat interval in milliseconds at which leaders will send heartbeats to followers
    #[clap(long, default_value = "50")]
    pub heartbeat_interval: u64,

    /// The timeout for sending then installing the last snapshot segment,
    /// in millisecond. It is also used as the timeout for sending a non-last segment, if
    /// `send_snapshot_timeout` is 0.
    #[clap(long, default_value = "200")]
    pub install_snapshot_timeout: u64,

    /// The timeout for sending a **non-last** snapshot segment, in milliseconds.
    ///
    /// It is disabled by default, by setting it to `0`.
    /// The timeout for sending every segment is `install_snapshot_timeout`.
    #[deprecated(
        since = "0.9.0",
        note = "Sending snapshot by chunks is deprecated; Use `install_snapshot_timeout` instead"
    )]
    #[clap(long, default_value = "0")]
    pub send_snapshot_timeout: u64,

    /// The maximum number of entries per payload allowed to be transmitted during replication
    ///
    /// If this is too low, it will take longer for the nodes to be brought up to
    /// consistency with the rest of the cluster.
    #[clap(long, default_value = "300")]
    pub max_payload_entries: u64,

    /// The distance behind in log replication a follower must fall before it is considered lagging
    ///
    /// A follower falls behind this index are replicated with snapshot.
    /// A follower falls within this index are replicated with log entries.
    ///
    /// This value should be greater than snapshot_policy.SnapshotPolicy.LogsSinceLast, otherwise
    /// transmitting a snapshot may not fix the lagging.
    #[clap(long, default_value = "5000")]
    pub replication_lag_threshold: u64,

    /// The snapshot policy to use for a Raft node.
    #[clap(
        long,
        default_value = "since_last:5000",
        value_parser=parse_snapshot_policy
    )]
    pub snapshot_policy: SnapshotPolicy,

    /// The maximum snapshot chunk size allowed when transmitting snapshots (in bytes)
    #[clap(long, default_value = "3MiB", value_parser=parse_bytes_with_unit)]
    pub snapshot_max_chunk_size: u64,

    /// The maximum number of logs to keep that are already included in **snapshot**.
    ///
    /// Logs that are not in snapshot will never be purged.
    #[clap(long, default_value = "1000")]
    pub max_in_snapshot_log_to_keep: u64,

    /// The minimal number of applied logs to purge in a batch.
    #[clap(long, default_value = "1")]
    pub purge_batch_size: u64,

    /// Enable or disable tick.
    ///
    /// If ticking is disabled, timeout based events are all disabled:
    /// a follower won't wake up to enter candidate state,
    /// and a leader won't send heartbeat.
    ///
    /// This flag is mainly used for test, or to build a consensus system that does not depend on
    /// wall clock. The value of this config is evaluated as follow:
    /// - being absent: true
    /// - `--enable-tick`: true
    /// - `--enable-tick=true`: true
    /// - `--enable-tick=false`: false
    // clap 4 requires `num_args = 0..=1`, or it complains about missing arg error
    // https://github.com/clap-rs/clap/discussions/4374
    #[clap(long,
           default_value_t = true,
           action = clap::ArgAction::Set,
           num_args = 0..=1,
           default_missing_value = "true"
    )]
    pub enable_tick: bool,

    /// Whether a leader sends heartbeat log to following nodes, i.e., followers and learners.
    // clap 4 requires `num_args = 0..=1`, or it complains about missing arg error
    // https://github.com/clap-rs/clap/discussions/4374
    #[clap(long,
           default_value_t = true,
           action = clap::ArgAction::Set,
           num_args = 0..=1,
           default_missing_value = "true"
    )]
    pub enable_heartbeat: bool,

    /// Whether a follower will enter candidate state if it does not receive message from the
    /// leader for a while.
    // clap 4 requires `num_args = 0..=1`, or it complains about missing arg error
    // https://github.com/clap-rs/clap/discussions/4374
    #[clap(long,
           default_value_t = true,
           action = clap::ArgAction::Set,
           num_args = 0..=1,
           default_missing_value = "true"
    )]
    pub enable_elect: bool,

    /// Whether to allow to reset the replication progress to `None`, when the
    /// follower's log is found reverted to an early state. **Do not enable this in production**
    /// unless you know what you are doing.
    ///
    /// Although log state reversion is typically seen as a bug, enabling it can be
    /// useful for testing or other special scenarios.
    /// For instance, in an even number nodes cluster, erasing a node's data and then
    /// rebooting it(log reverts to empty) will not result in data loss.
    ///
    /// For one-shot log reversion, use
    /// [`Raft::trigger().allow_next_revert()`](crate::raft::trigger::Trigger::allow_next_revert).
    ///
    /// Since: 0.10.0
    #[clap(long,
           action = clap::ArgAction::Set,
           num_args = 0..=1,
           default_missing_value = "true"
    )]
    pub allow_log_reversion: Option<bool>,
}

/// Updatable config for a raft runtime.
pub(crate) struct RuntimeConfig {
    pub(crate) enable_heartbeat: AtomicBool,
    pub(crate) enable_elect: AtomicBool,
}

impl RuntimeConfig {
    pub(crate) fn new(config: &Config) -> Self {
        Self {
            enable_heartbeat: AtomicBool::from(config.enable_heartbeat),
            enable_elect: AtomicBool::from(config.enable_elect),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        <Self as Parser>::parse_from(Vec::<&'static str>::new())
    }
}

impl Config {
    /// Generate a new random election timeout within the configured min & max.
    pub fn new_rand_election_timeout<RT: AsyncRuntime>(&self) -> u64 {
        RT::thread_rng().random_range(self.election_timeout_min..self.election_timeout_max)
    }

    /// Get the timeout for sending and installing the last snapshot segment.
    pub fn install_snapshot_timeout(&self) -> Duration {
        Duration::from_millis(self.install_snapshot_timeout)
    }

    /// Get the timeout for sending a non-last snapshot segment.
    #[deprecated(
        since = "0.9.0",
        note = "Sending snapshot by chunks is deprecated; Use `install_snapshot_timeout()` instead"
    )]
    pub fn send_snapshot_timeout(&self) -> Duration {
        #[allow(deprecated)]
        if self.send_snapshot_timeout > 0 {
            Duration::from_millis(self.send_snapshot_timeout)
        } else {
            self.install_snapshot_timeout()
        }
    }

    /// Whether to allow the replication to reset the state to `None` when a log state reversion is
    /// detected.
    ///
    /// By default, it does not allow log reversion, because it might indicate a bug in the system.
    pub(crate) fn get_allow_log_reversion(&self) -> bool {
        self.allow_log_reversion.unwrap_or(false)
    }

    /// Build a `Config` instance from a series of command line arguments.
    ///
    /// The first element in `args` must be the application name.
    pub fn build(args: &[&str]) -> Result<Config, ConfigError> {
        let config = <Self as Parser>::try_parse_from(args).map_err(|e| ConfigError::ParseError {
            source: AnyError::from(&e),
            args: args.iter().map(|x| x.to_string()).collect(),
        })?;
        config.validate()
    }

    /// Validate the state of this config.
    pub fn validate(self) -> Result<Config, ConfigError> {
        if self.election_timeout_min >= self.election_timeout_max {
            return Err(ConfigError::ElectionTimeout {
                min: self.election_timeout_min,
                max: self.election_timeout_max,
            });
        }

        if self.election_timeout_min <= self.heartbeat_interval {
            return Err(ConfigError::ElectionTimeoutLTHeartBeat {
                election_timeout_min: self.election_timeout_min,
                heartbeat_interval: self.heartbeat_interval,
            });
        }

        if self.max_payload_entries == 0 {
            return Err(ConfigError::MaxPayloadIs0);
        }

        Ok(self)
    }
}
