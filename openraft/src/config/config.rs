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
use crate::config::StepDownPolicy;
use crate::config::error::ConfigError;
#[cfg(feature = "clap")]
use crate::config::parser::parse_bytes_with_unit;
#[cfg(feature = "clap")]
use crate::config::parser::parse_snapshot_policy;
#[cfg(feature = "clap")]
use crate::config::parser::parse_step_down_policy;
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
    pub heartbeat_min_interval: u64,
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
    pub api_batch_capacity: u64,
    pub api_batch_linger_ms: u64,
    pub notification_channel_size: u64,
    pub state_machine_channel_size: u64,
    pub backoff: &'static str,
    pub enable_tick: bool,
    pub enable_heartbeat: bool,
    pub enable_elect: bool,
    pub removed_leader_step_down: StepDownPolicy,
    pub enable_pre_vote: Option<bool>,
}

pub(crate) const DEFAULTS: Defaults = Defaults {
    cluster_name: "foo",
    election_timeout_min: 150,
    election_timeout_max: 300,
    heartbeat_interval: 50,
    heartbeat_min_interval: 0,
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
    api_batch_capacity: 4096,
    api_batch_linger_ms: 0,
    notification_channel_size: 65536,
    state_machine_channel_size: 1024,
    backoff: "200ms",
    enable_tick: true,
    enable_heartbeat: true,
    enable_elect: true,
    removed_leader_step_down: StepDownPolicy::After(150),
    enable_pre_vote: None,
};

/// The serde default for [`Config::removed_leader_step_down`]: it is used when the field is
/// absent, so that config files written before this field existed keep the default behavior.
#[cfg(feature = "serde")]
fn default_removed_leader_step_down() -> StepDownPolicy {
    DEFAULTS.removed_leader_step_down.clone()
}

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
#[since]
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

    /// The minimum interval in milliseconds between two heartbeats to the same follower.
    ///
    /// A successful replication response proves the same liveness facts as a heartbeat response
    /// and also updates the leader lease clock. When a follower has acknowledged an RPC
    /// (replication or heartbeat) that was sent within the last `heartbeat_min_interval`
    /// milliseconds, the periodic heartbeat to this follower is suppressed, since it would be
    /// redundant. Under sustained writes this eliminates most heartbeat RPCs; heartbeats resume
    /// automatically once replication idles.
    ///
    /// It must hold that `heartbeat_interval + heartbeat_min_interval < election_timeout_min`,
    /// so that suppression can never delay a heartbeat past a follower's election timeout.
    ///
    /// Defaults to 0, meaning heartbeats are always sent at `heartbeat_interval`.
    #[since(version = "0.10.0")]
    #[cfg_attr(feature = "clap", clap(long, default_value = "0"))]
    pub heartbeat_min_interval: Option<u64>,

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
    #[since(version = "0.10.0")]
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
    #[since(version = "0.10.0")]
    #[cfg_attr(feature = "clap", clap(long, default_value = "65536"))]
    pub api_channel_size: Option<u64>,

    /// `RaftMsg::ClientWrite` batch before it is processed.
    /// Then the batched `ClientWrite` requests will be submitted to `RaftStorage` in one shot.
    #[since(version = "0.10.0")]
    #[cfg_attr(feature = "clap", clap(long, default_value = "4096"))]
    pub api_batch_capacity: u64,

    /// Maximum amount of milliseconds to wait for additional client requests before
    /// flushing a partially filled batch.
    #[since(version = "0.10.0")]
    #[cfg_attr(feature = "clap", clap(long, default_value = "0"))]
    pub api_batch_linger_ms: u64,
    /// The size of the bounded notification channel for internal events.
    ///
    /// This channel carries internal notifications like IO completion, replication progress,
    /// and tick events. When full, internal components will block until space is available.
    #[since(version = "0.10.0")]
    #[cfg_attr(feature = "clap", clap(long, default_value = "65536"))]
    pub notification_channel_size: Option<u64>,

    /// The size of the bounded channel for sending commands to the state machine worker.
    ///
    /// This channel carries commands like Apply, BuildSnapshot, and InstallSnapshot.
    /// When full, RaftCore will block until space becomes available, providing backpressure
    /// when the state machine is slow.
    #[since(version = "0.10.0")]
    #[cfg_attr(feature = "clap", clap(long, default_value = "1024"))]
    pub state_machine_channel_size: Option<u64>,

    /// The capacity of the ring buffer used to track lifecycle latency per stage.
    ///
    /// Each of the 6 lifecycle stages (proposed, received, submitted, persisted,
    /// committed, applied) maintains a fixed-capacity ring buffer of this size.
    /// Only used when the `runtime-stats` feature is enabled.
    ///
    /// Defaults to 1024 if not specified.
    #[since(version = "0.10.0")]
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

    /// The policy for stepping down a Leader that is removed from a committed membership config.
    ///
    /// - [`After(ms)`](StepDownPolicy::After): when the membership config that removes this Leader
    ///   is committed and `ms` milliseconds have elapsed, the Leader transfers leadership to the
    ///   most up-to-date voter, then reverts itself to a learner. During the delay it keeps serving
    ///   as a Leader, so that in-flight requests and the commit notification of the membership log
    ///   entry can still reach the followers.
    /// - [`Never`](StepDownPolicy::Never): the removed Leader keeps leading, until the application
    ///   reverts it manually with [`Trigger::refresh_server_state()`].
    ///
    /// In CLI it is either a "never" literal (`never`, `no`, `none`, `off` or `false`,
    /// case-insensitive) or the number of milliseconds, e.g.,
    /// `--removed-leader-step-down=never` or `--removed-leader-step-down=150`.
    ///
    /// Defaults to `After(150)`.
    ///
    /// [`Trigger::refresh_server_state()`]: crate::raft::trigger::Trigger::refresh_server_state
    #[since(version = "0.10.0")]
    #[cfg_attr(feature = "clap", clap(long, default_value = "150", value_parser = parse_step_down_policy))]
    #[cfg_attr(feature = "serde", serde(default = "default_removed_leader_step_down"))]
    pub removed_leader_step_down: StepDownPolicy,

    /// Whether a follower runs a Pre-Vote round before incrementing its term and starting a real
    /// election.
    ///
    /// When enabled (`Some(true)`), a follower whose election timer fires first asks peers whether
    /// they *would* grant it a vote at `term + 1`, without persisting any vote or bumping its term.
    /// Only after a quorum would grant does it run the real election. This prevents a node that
    /// cannot currently win — e.g. one that was partitioned, restarted, or has a stale log — from
    /// inflating its term and disrupting a healthy leader once it reconnects.
    ///
    /// When disabled (`Some(false)`), a follower increments its term and votes for itself
    /// immediately on election timeout, the historical Openraft behavior. The leader-lease already
    /// rejects such a candidate's vote requests, so Pre-Vote is an optional refinement rather than
    /// a correctness requirement.
    ///
    /// `None` (the default) leaves the choice to Openraft, which currently treats it as disabled.
    /// Leaving it unset lets a future release change this default without breaking configs that
    /// never opted in.
    ///
    /// Pre-Vote uses a separate network RPC
    /// ([`RaftNetworkV2::pre_vote`](crate::network::v2::RaftNetworkV2::pre_vote)). A peer whose
    /// network does not implement it is counted as granting the Pre-Vote, so a cluster mid-upgrade
    /// stays live.
    #[since(version = "0.10.0")]
    // clap 4 requires `num_args = 0..=1`, or it complains about missing arg error
    // https://github.com/clap-rs/clap/discussions/4374
    #[cfg_attr(feature = "clap", clap(long,
           action = clap::ArgAction::Set,
           num_args = 0..=1,
           default_missing_value = "true"
    ))]
    pub enable_pre_vote: Option<bool>,

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
    #[since(version = "0.10.0")]
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
    #[since(version = "0.10.0")]
    #[cfg_attr(feature = "clap", clap(long,
           action = clap::ArgAction::Set,
           num_args = 0..=1,
           default_missing_value = "true"
    ))]
    pub allow_log_reversion: Option<bool>,

    /// Whether to allow a restarted leader to restore leadership without a vote.
    ///
    /// When enabled (`true`), if a node that was leader before a restart comes back quickly
    /// enough, before any other node triggers and election, it can continue serving as leader
    /// without re-election. This differs from standard Raft, but can help improve availability
    /// after a leader restart.
    ///
    /// When disabled (`false`), a restarted leader must always trigger a new election to become
    /// leader again.
    ///
    /// **Important**: When this setting is enabled, it can introduce inconsistencies when the
    /// following conditions are true:
    ///
    /// - The state machine does not flush state to disk before returning from
    ///   [`openraft::storage::RaftLogStorage::apply`].
    /// - The last committed log id is not persisted.
    ///
    /// When the above conditions are met and this setting is enabled, then a leader can restart
    /// with a stale state machine and restore itself as leader without any mechanism to detect
    /// the staleness.
    ///
    /// When the above conditions are met and this setting is disabled, then a restarted leader can
    /// learn the current commit log id by participating in a new election or receiving a message
    /// from the new leader.
    ///
    /// See: [`docs::data::log_pointers`].
    ///
    /// [`docs::data::log_pointers`]: `crate::docs::data::log_pointers#optionally-persisted-committed`
    // clap 4 requires `num_args = 0..=1`, or it complains about missing arg error
    // https://github.com/clap-rs/clap/discussions/4374
    #[since(version = "0.10.0")]
    #[cfg_attr(feature = "clap", clap(long,
           action = clap::ArgAction::Set,
           num_args = 0..=1,
           default_missing_value = "true"
    ))]
    pub enable_leader_restore: Option<bool>,
}

impl Default for Config {
    #[allow(deprecated)]
    fn default() -> Self {
        Self {
            cluster_name: DEFAULTS.cluster_name.to_string(),
            election_timeout_min: DEFAULTS.election_timeout_min,
            election_timeout_max: DEFAULTS.election_timeout_max,
            heartbeat_interval: DEFAULTS.heartbeat_interval,
            heartbeat_min_interval: Some(DEFAULTS.heartbeat_min_interval),
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
            api_batch_capacity: DEFAULTS.api_batch_capacity,
            api_batch_linger_ms: DEFAULTS.api_batch_linger_ms,
            notification_channel_size: Some(DEFAULTS.notification_channel_size),
            state_machine_channel_size: Some(DEFAULTS.state_machine_channel_size),
            log_stage_capacity: None,
            enable_tick: DEFAULTS.enable_tick,
            enable_heartbeat: DEFAULTS.enable_heartbeat,
            enable_elect: DEFAULTS.enable_elect,
            removed_leader_step_down: DEFAULTS.removed_leader_step_down.clone(),
            enable_pre_vote: DEFAULTS.enable_pre_vote,
            backoff: DEFAULTS.backoff.to_string(),
            allow_log_reversion: None,
            enable_leader_restore: None,
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

    /// Whether a node that was a leader before a restart restores leadership at startup, without
    /// an election.
    ///
    /// By default, it restores leadership at once for better availability.
    #[since(version = "0.10.0")]
    pub(crate) fn enable_leader_restore(&self) -> bool {
        self.enable_leader_restore.unwrap_or(true)
    }

    /// Whether a follower runs a Pre-Vote round before starting a real election.
    ///
    /// Evaluates the [`enable_pre_vote`](Self::enable_pre_vote) option: `None` is treated as
    /// disabled (`false`), the current default.
    pub(crate) fn get_enable_pre_vote(&self) -> bool {
        self.enable_pre_vote.unwrap_or(false)
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

    /// Get the minimum interval in milliseconds between two heartbeats to the same follower.
    ///
    /// Defaults to 0 if not specified, meaning heartbeat suppression is disabled.
    pub(crate) fn heartbeat_min_interval(&self) -> u64 {
        self.heartbeat_min_interval.unwrap_or(0)
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

        if self.election_timeout_min <= self.heartbeat_interval + self.heartbeat_min_interval() {
            return Err(ConfigError::HeartbeatMinIntervalTooLarge {
                election_timeout_min: self.election_timeout_min,
                heartbeat_interval: self.heartbeat_interval,
                heartbeat_min_interval: self.heartbeat_min_interval(),
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
