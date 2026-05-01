//! Raft runtime configuration.

use std::ops::Deref;
use std::time::Duration;

use backoff_series::BackoffSeries;
#[cfg(feature = "clap")]
use clap::Parser;
use openraft_macros::since;
use rand::RngExt;

use crate::AsyncRuntime;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::config::error::ConfigError;
#[cfg(feature = "clap")]
use crate::config::parser::parse_bytes_with_unit;
#[cfg(feature = "clap")]
use crate::config::parser::parse_snapshot_policy;
use crate::network::Backoff;
use crate::raft_state::LogStateReader;
use crate::vote::RaftCommittedLeaderId;

/// Default values for [`Config`] fields.
///
/// Shared between the [`Default`] impl and (when enabled) the clap
/// `default_value_t` / `default_value` attributes, so the two stay in sync.
pub(crate) struct Defaults {
    pub cluster_name: &'static str,
    pub election_timeout_min: u64,
    pub election_timeout_max: u64,
    pub heartbeat_interval: u64,
    pub install_snapshot_timeout: u64,
    pub send_snapshot_timeout: u64,
    pub max_payload_entries: u64,
    pub max_append_entries: u64,
    pub replication_lag_threshold: u64,
    pub snapshot_policy: SnapshotPolicy,
    pub snapshot_max_chunk_size: u64,
    pub max_in_snapshot_log_to_keep: u64,
    pub purge_batch_size: u64,
    pub api_channel_size: u64,
    pub notification_channel_size: u64,
    pub state_machine_channel_size: u64,
    pub backoff: &'static str,
    pub enable_tick: bool,
    pub enable_heartbeat: bool,
    pub enable_elect: bool,
}

pub(crate) const DEFAULTS: Defaults = Defaults {
    cluster_name: "foo",
    election_timeout_min: 150,
    election_timeout_max: 300,
    heartbeat_interval: 50,
    install_snapshot_timeout: 200,
    send_snapshot_timeout: 0,
    max_payload_entries: 300,
    max_append_entries: 4096,
    replication_lag_threshold: 5000,
    snapshot_policy: SnapshotPolicy::LogsSinceLast(5000),
    snapshot_max_chunk_size: 3 * 1024 * 1024,
    max_in_snapshot_log_to_keep: 1000,
    purge_batch_size: 1,
    api_channel_size: 65536,
    notification_channel_size: 65536,
    state_machine_channel_size: 1024,
    backoff: "200ms",
    enable_tick: true,
    enable_heartbeat: true,
    enable_elect: true,
};

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
    pub(crate) fn should_snapshot<CLID>(
        &self,
        state: &impl Deref<Target = impl LogStateReader<CLID>>,
        last_tried_at: Option<&LogId<CLID>>,
    ) -> Option<LogId<CLID>>
    where
        CLID: RaftCommittedLeaderId,
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
#[derive(Clone, Debug)]
#[cfg_attr(feature = "clap", derive(Parser))]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct Config {
    /// The application-specific name of this Raft cluster
    #[cfg_attr(feature = "clap", clap(long, default_value = DEFAULTS.cluster_name))]
    pub cluster_name: String,

    /// The minimum election timeout in milliseconds
    #[cfg_attr(feature = "clap", clap(long, default_value_t = DEFAULTS.election_timeout_min))]
    pub election_timeout_min: u64,

    /// The maximum election timeout in milliseconds
    #[cfg_attr(feature = "clap", clap(long, default_value_t = DEFAULTS.election_timeout_max))]
    pub election_timeout_max: u64,

    /// The heartbeat interval in milliseconds at which leaders will send heartbeats to followers
    #[cfg_attr(feature = "clap", clap(long, default_value_t = DEFAULTS.heartbeat_interval))]
    pub heartbeat_interval: u64,

    /// The timeout for sending then installing the last snapshot segment,
    /// in millisecond. It is also used as the timeout for sending a non-last segment if
    /// `send_snapshot_timeout` is 0.
    #[cfg_attr(feature = "clap", clap(long, default_value_t = DEFAULTS.install_snapshot_timeout))]
    pub install_snapshot_timeout: u64,

    /// The timeout for sending a **non-last** snapshot segment, in milliseconds.
    ///
    /// It is disabled by default by setting it to `0`.
    /// The timeout for sending every segment is `install_snapshot_timeout`.
    #[deprecated(
        since = "0.9.0",
        note = "Sending snapshot by chunks is deprecated; Use `install_snapshot_timeout` instead"
    )]
    #[cfg_attr(feature = "clap", clap(long, default_value_t = DEFAULTS.send_snapshot_timeout))]
    pub send_snapshot_timeout: u64,

    /// The maximum number of entries per payload allowed to be transmitted during replication
    ///
    /// If this is too low, it will take longer for the nodes to be brought up to
    /// consistency with the rest of the cluster.
    #[cfg_attr(feature = "clap", clap(long, default_value_t = DEFAULTS.max_payload_entries))]
    pub max_payload_entries: u64,

    /// The maximum number of log entries per append I/O operation.
    ///
    /// When multiple `AppendEntries` commands are queued, Openraft can merge them into
    /// a single storage write to improve throughput. This setting limits the batch size
    /// to prevent excessively large writes that could cause high storage latency.
    ///
    /// This is separate from `max_payload_entries` which controls network replication payload size.
    /// Storage typically has higher throughput than network, so this value can be larger.
    ///
    /// Defaults to 4096.
    #[cfg_attr(feature = "clap", clap(long, default_value = "4096"))]
    pub max_append_entries: Option<u64>,

    /// The distance behind in log replication a follower must fall before it is considered lagging
    ///
    /// - Followers that fall behind this index are replicated with a snapshot.
    /// - Followers that fall within this index are replicated with log entries.
    ///
    /// This value should be greater than snapshot_policy.SnapshotPolicy.LogsSinceLast, otherwise
    /// transmitting a snapshot may not fix the lagging.
    #[cfg_attr(feature = "clap", clap(long, default_value_t = DEFAULTS.replication_lag_threshold))]
    pub replication_lag_threshold: u64,

    /// The snapshot policy to use for a Raft node.
    #[cfg_attr(feature = "clap", clap(
        long,
        default_value = "since_last:5000",
        value_parser=parse_snapshot_policy
    ))]
    pub snapshot_policy: SnapshotPolicy,

    /// The maximum snapshot chunk size allowed when transmitting snapshots (in bytes)
    #[cfg_attr(feature = "clap", clap(long, default_value = "3MiB", value_parser=parse_bytes_with_unit))]
    pub snapshot_max_chunk_size: u64,

    /// The maximum number of logs to keep that are already included in **snapshot**.
    ///
    /// Logs that are not in a snapshot will never be purged.
    #[cfg_attr(feature = "clap", clap(long, default_value_t = DEFAULTS.max_in_snapshot_log_to_keep))]
    pub max_in_snapshot_log_to_keep: u64,

    /// The minimal number of applied logs to purge in a batch.
    #[cfg_attr(feature = "clap", clap(long, default_value_t = DEFAULTS.purge_batch_size))]
    pub purge_batch_size: u64,

    /// The size of the bounded API channel for sending messages to RaftCore.
    ///
    /// This controls backpressure for client requests. When the channel is full,
    /// new API calls will block until space becomes available.
    #[cfg_attr(feature = "clap", clap(long, default_value = "65536"))]
    pub api_channel_size: Option<u64>,

    /// The size of the bounded notification channel for internal events.
    ///
    /// This channel carries internal notifications like IO completion, replication progress,
    /// and tick events. When full, internal components will block until space is available.
    #[cfg_attr(feature = "clap", clap(long, default_value = "65536"))]
    pub notification_channel_size: Option<u64>,

    /// The size of the bounded channel for sending commands to the state machine worker.
    ///
    /// This channel carries commands like Apply, BuildSnapshot, and InstallSnapshot.
    /// When full, RaftCore will block until space becomes available, providing backpressure
    /// when the state machine is slow.
    #[cfg_attr(feature = "clap", clap(long, default_value = "1024"))]
    pub state_machine_channel_size: Option<u64>,

    /// The capacity of the ring buffer used to track lifecycle latency per stage.
    ///
    /// Each of the 6 lifecycle stages (proposed, received, submitted, persisted,
    /// committed, applied) maintains a fixed-capacity ring buffer of this size.
    /// Only used when the `runtime-stats` feature is enabled.
    ///
    /// Defaults to 1024 if not specified.
    #[cfg_attr(feature = "clap", clap(long))]
    pub log_stage_capacity: Option<u64>,

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
    #[cfg_attr(feature = "clap", clap(long,
           default_value_t = true,
           action = clap::ArgAction::Set,
           num_args = 0..=1,
           default_missing_value = "true"
    ))]
    pub enable_tick: bool,

    /// Whether a leader sends heartbeat logs to following nodes, i.e., followers and learners.
    // clap 4 requires `num_args = 0..=1`, or it complains about missing arg error
    // https://github.com/clap-rs/clap/discussions/4374
    #[cfg_attr(feature = "clap", clap(long,
           default_value_t = true,
           action = clap::ArgAction::Set,
           num_args = 0..=1,
           default_missing_value = "true"
    ))]
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
    #[cfg_attr(feature = "clap", clap(long,
           default_value_t = true,
           action = clap::ArgAction::Set,
           num_args = 0..=1,
           default_missing_value = "true"
    ))]
    pub enable_elect: bool,

    /// Default backoff policy used when
    /// [`RaftNetworkV2::backoff`](crate::network::RaftNetworkV2::backoff) returns `None`.
    ///
    /// The string lists the first few retry delays explicitly. After those, the sequence
    /// continues automatically — smoothly extended from **at most the last three explicit
    /// delays** by either an exponential `k · aˣ + t` or linear `k · x + t` form, whichever
    /// fits the tail. A trailing `..<max>` or `...<max>` caps every extrapolated delay at
    /// `<max>` (defaults to `1000ms`).
    ///
    /// # Syntax
    ///
    /// `<dur>` values separated by `,` or whitespace; an optional `..` or `...` before the
    /// last value marks it as the **max cap**. `<dur>` is a number with a unit suffix:
    /// `ns`, `us`, `ms`, `s`, `m`, `h`.
    ///
    /// # Examples
    ///
    /// **Doubling from a single anchor** — this is the default, `"200ms"`:
    ///
    /// ```text
    /// 200  400  800  1000  1000  1000  …    (ms; cap = 1000)
    /// ```
    ///
    /// **Exponential growth**, `"100ms 200ms 400ms ...5s"`:
    ///
    /// ```text
    /// 100  200  400  800  1600  3200  5000  5000  …    (ms; cap = 5000)
    /// ```
    ///
    /// **Linear growth**, `"1ms 2ms 3ms ...1500ms"` — the tail has constant difference, so the
    /// sequence continues that line until it hits the cap:
    ///
    /// ```text
    /// 1  2  3  4  5  6  …  1499  1500  1500  1500  …    (ms; cap = 1500)
    /// ```
    ///
    /// **Constant delay**, `"500ms 500ms 500ms"` — a flat tail stays flat:
    ///
    /// ```text
    /// 500  500  500  500  …    (ms)
    /// ```
    ///
    /// **Short warm-up, then extrapolate**, `"10ms 20ms ...2s"` — two anchors imply a doubling
    /// rate, extended automatically:
    ///
    /// ```text
    /// 10  20  40  80  160  320  640  1280  2000  2000  …    (ms; cap = 2000)
    /// ```
    ///
    /// Because only the last few explicit delays shape what follows, you can prepend any
    /// number of custom warm-up values without changing the long-run behavior.
    ///
    /// Since: 0.10.0
    #[cfg_attr(feature = "clap", clap(long, default_value = DEFAULTS.backoff))]
    pub backoff: String,

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
    #[cfg_attr(feature = "clap", clap(long,
           action = clap::ArgAction::Set,
           num_args = 0..=1,
           default_missing_value = "true"
    ))]
    pub allow_log_reversion: Option<bool>,
}

impl Default for Config {
    #[allow(deprecated)]
    fn default() -> Self {
        Self {
            cluster_name: DEFAULTS.cluster_name.to_string(),
            election_timeout_min: DEFAULTS.election_timeout_min,
            election_timeout_max: DEFAULTS.election_timeout_max,
            heartbeat_interval: DEFAULTS.heartbeat_interval,
            install_snapshot_timeout: DEFAULTS.install_snapshot_timeout,
            send_snapshot_timeout: DEFAULTS.send_snapshot_timeout,
            max_payload_entries: DEFAULTS.max_payload_entries,
            max_append_entries: Some(DEFAULTS.max_append_entries),
            replication_lag_threshold: DEFAULTS.replication_lag_threshold,
            snapshot_policy: DEFAULTS.snapshot_policy.clone(),
            snapshot_max_chunk_size: DEFAULTS.snapshot_max_chunk_size,
            max_in_snapshot_log_to_keep: DEFAULTS.max_in_snapshot_log_to_keep,
            purge_batch_size: DEFAULTS.purge_batch_size,
            api_channel_size: Some(DEFAULTS.api_channel_size),
            notification_channel_size: Some(DEFAULTS.notification_channel_size),
            state_machine_channel_size: Some(DEFAULTS.state_machine_channel_size),
            log_stage_capacity: None,
            enable_tick: DEFAULTS.enable_tick,
            enable_heartbeat: DEFAULTS.enable_heartbeat,
            enable_elect: DEFAULTS.enable_elect,
            backoff: DEFAULTS.backoff.to_string(),
            allow_log_reversion: None,
        }
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

    /// Get the state machine command channel size for bounded MPSC channel.
    ///
    /// Defaults to 1024 if not specified.
    pub(crate) fn state_machine_channel_size(&self) -> usize {
        self.state_machine_channel_size.unwrap_or(1024) as usize
    }

    /// Get the lifecycle latency ring buffer capacity per stage.
    ///
    /// Defaults to 1024 if not specified.
    #[allow(dead_code)]
    pub(crate) fn log_stage_capacity(&self) -> usize {
        self.log_stage_capacity.unwrap_or(1024) as usize
    }

    /// Get the maximum number of log entries per append I/O operation.
    ///
    /// Defaults to 4096 if not specified.
    pub(crate) fn max_append_entries(&self) -> u64 {
        self.max_append_entries.unwrap_or(4096)
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

        // Validate the backoff policy string up-front so build_backoff() can assume it parses.
        BackoffSeries::parse(&self.backoff)?;

        Ok(self)
    }

    /// Build a [`Backoff`] iterator from the [`backoff`](Self::backoff) policy string.
    ///
    /// Panics if the policy is invalid — callers should obtain `Config` through [`Config::build`]
    /// or [`Config::validate`], which reject invalid policies up-front.
    #[since(version = "0.10.0")]
    pub fn build_backoff(&self) -> Backoff {
        let series = BackoffSeries::parse(&self.backoff).expect("backoff policy must be validated by Config::validate");
        Backoff::new(series.build())
    }
}
