//! Raft runtime configuration.

use std::ops::Deref;
use std::str::FromStr;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use anyerror::AnyError;
use clap::Parser;
use openraft_macros::since;
use rand::Rng;

use crate::AsyncRuntime;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::RaftTypeConfig;
use crate::config::error::ConfigError;
use crate::raft_state::LogStateReader;

/// Log compaction and snapshot policy.
///
/// This governs when periodic snapshots will be taken, as well as the conditions which
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
    pub(crate) fn should_snapshot<C>(
        &self,
        state: &impl Deref<Target = impl LogStateReader<C>>,
        last_tried_at: Option<&LogId<C>>,
    ) -> Option<LogId<C>>
    where
        C: RaftTypeConfig,
    {
        match self {
            SnapshotPolicy::LogsSinceLast(threshold) => {
                let committed_next = state.committed().next_index();
                let base_log_id = last_tried_at.max(state.snapshot_last_log_id());

                if committed_next >= base_log_id.next_index() + threshold {
                    state.committed().cloned()
                } else {
                    None
                }
            }
            SnapshotPolicy::Never => None,
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

/// Runtime configuration for a Raft node.
///
/// `Config` controls tunable parameters for Raft operation including election timeouts, heartbeat
/// intervals, replication settings, snapshot policies, and log compaction behavior.
///
/// # Usage
///
/// Create a configuration, optionally customize fields, validate it, and pass to [`Raft::new`]:
///
/// ```ignore
/// use openraft::Config;
/// use std::sync::Arc;
///
/// let config = Config {
///     cluster_name: "my-cluster".to_string(),
///     heartbeat_interval: 50,
///     election_timeout_min: 150,
///     election_timeout_max: 300,
///     ..Default::default()
/// };
///
/// let config = Arc::new(config.validate()?);
/// let raft = Raft::new(node_id, config, network, log_store, state_machine).await?;
/// ```
///
/// # Timing Constraints
///
/// Follow the Raft timing inequality: `broadcastTime ≪ electionTimeout ≪ MTBF`
///
/// **Rule of thumb**: Set `heartbeat_interval ≈ election_timeout / 3` and ensure election timeout
/// is 10-20× your typical network round-trip time.
///
/// # See Also
///
/// - [Raft specification §5.6](https://raft.github.io/raft.pdf) for timing guidance
/// - [`SnapshotPolicy`] for snapshot triggering strategies
///
/// [`Raft::new`]: crate::Raft::new
#[derive(Clone, Debug, Parser)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct Config {
    /// The application-specific name of this Raft cluster
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
    /// in millisecond. It is also used as the timeout for sending a non-last segment if
    /// `send_snapshot_timeout` is 0.
    #[clap(long, default_value = "200")]
    pub install_snapshot_timeout: u64,

    /// The timeout for sending a **non-last** snapshot segment, in milliseconds.
    ///
    /// It is disabled by default by setting it to `0`.
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
    /// - Followers that fall behind this index are replicated with a snapshot.
    /// - Followers that fall within this index are replicated with log entries.
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
    /// Logs that are not in a snapshot will never be purged.
    #[clap(long, default_value = "1000")]
    pub max_in_snapshot_log_to_keep: u64,

    /// The minimal number of applied logs to purge in a batch.
    #[clap(long, default_value = "1")]
    pub purge_batch_size: u64,

    /// The size of the bounded API channel for sending messages to RaftCore.
    ///
    /// This controls backpressure for client requests. When the channel is full,
    /// new API calls will block until space becomes available.
    #[clap(long, default_value = "65536")]
    pub api_channel_size: Option<u64>,

    /// The size of the bounded notification channel for internal events.
    ///
    /// This channel carries internal notifications like IO completion, replication progress,
    /// and tick events. When full, internal components will block until space is available.
    #[clap(long, default_value = "65536")]
    pub notification_channel_size: Option<u64>,

    /// Enable or disable tick.
    ///
    /// If ticking is disabled, timeout-based events are all disabled:
    /// a follower won't wake up to enter candidate state,
    /// and a leader won't send heartbeat.
    ///
    /// **Critical for elections**: When `false`, followers cannot detect leader failures and will
    /// never trigger elections, even with `enable_elect = true`. The tick mechanism provides the
    /// timing infrastructure that allows followers to detect when `election_timeout_max` has passed
    /// without receiving heartbeats from the leader.
    ///
    /// This flag is mainly used for testing or to build a consensus system that does not depend on
    /// wall clock. The value of this config is evaluated as follows:
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

    /// Whether a leader sends heartbeat logs to following nodes, i.e., followers and learners.
    // clap 4 requires `num_args = 0..=1`, or it complains about missing arg error
    // https://github.com/clap-rs/clap/discussions/4374
    #[clap(long,
           default_value_t = true,
           action = clap::ArgAction::Set,
           num_args = 0..=1,
           default_missing_value = "true"
    )]
    pub enable_heartbeat: bool,

    /// Whether a follower will enter candidate state if it does not receive any messages from the
    /// leader for a while.
    ///
    /// When enabled (`true`), followers automatically trigger elections by entering the `Candidate`
    /// state when they don't receive `AppendEntries` messages (heartbeats) from the leader for
    /// longer than `election_timeout_max`. This is essential for automatic failover when a leader
    /// becomes unavailable.
    ///
    /// When disabled (`false`), followers will never initiate elections, even if the leader fails.
    /// This setting is primarily for testing or building custom consensus systems.
    ///
    /// **Important**: This setting only works when `enable_tick` is also `true`. Elections require
    /// time-based events to detect leader absence.
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

    /// Allow IO completion notifications to arrive out of order.
    ///
    /// When enabled, storage implementations may report IO completions non-monotonically.
    /// For example, flushed entries `[5..6)` may notify before `[6..8)`.
    ///
    /// Enable this if your storage does not guarantee strictly ordered notifications.
    ///
    /// Default: `false` (detects out-of-order bugs by default)
    ///
    /// Since: 0.10.0
    #[clap(long,
           action = clap::ArgAction::Set,
           num_args = 0..=1,
           default_missing_value = "true"
    )]
    pub allow_io_notification_reorder: Option<bool>,
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
    /// Generate a new random election timeout within the configured min and max values.
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
    #[since(version = "0.10.0")]
    pub(crate) fn get_allow_log_reversion(&self) -> bool {
        self.allow_log_reversion.unwrap_or(false)
    }

    /// Returns whether IO completion notifications can arrive out of order.
    ///
    /// Default: `false`
    #[since(version = "0.10.0")]
    pub(crate) fn get_allow_io_notification_reorder(&self) -> bool {
        self.allow_io_notification_reorder.unwrap_or(false)
    }

    /// Get the API channel size for bounded MPSC channel.
    ///
    /// Defaults to 65536 if not specified.
    pub(crate) fn api_channel_size(&self) -> usize {
        self.api_channel_size.unwrap_or(65536) as usize
    }

    /// Get the notification channel size for bounded MPSC channel.
    ///
    /// Defaults to 65536 if not specified.
    pub(crate) fn notification_channel_size(&self) -> usize {
        self.notification_channel_size.unwrap_or(65536) as usize
    }

    /// Build a `Config` instance from a series of command line arguments.
    ///
    /// The first element in `args` must be the application name.
    ///
    /// # Examples
    ///
    /// ```
    /// use openraft::Config;
    ///
    /// let config = Config::build(&[
    ///     "myapp",
    ///     "--election-timeout-min", "300",
    ///     "--election-timeout-max", "500",
    /// ])?;
    /// # Ok::<(), openraft::ConfigError>(())
    /// ```
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
