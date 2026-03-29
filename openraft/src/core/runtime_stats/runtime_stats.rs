use std::marker::PhantomData;

use base2histogram::Histogram;

use crate::Config;
use crate::RaftTypeConfig;
use crate::core::NotificationName;
use crate::core::raft_msg::RaftMsgName;
use crate::core::runtime_stats::DisplayMode;
use crate::core::runtime_stats::RuntimeStatsDisplay;
use crate::core::runtime_stats::log_stage::LogStageHistograms;
use crate::core::runtime_stats::log_stage::LogStages;
use crate::core::stage::Stage;
use crate::engine::CommandName;
#[cfg(feature = "runtime-stats")]
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::InstantOf;

/// Runtime statistics for Raft operations.
///
/// This is a volatile structure that is not persisted. It accumulates
/// statistics from the time the Raft node starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStats<C>
where C: RaftTypeConfig
{
    /// Histogram tracking the distribution of log entry counts in Apply commands.
    ///
    /// This tracks how many log entries are included in each apply command sent
    /// to the state machine, helping identify batch size patterns and I/O efficiency.
    pub apply_batch: Histogram,

    /// Histogram tracking the distribution of log entry counts when appending to storage.
    ///
    /// This tracks how many log entries are included in each AppendEntries command
    /// submitted to the storage layer, helping identify write batch patterns and storage I/O
    /// efficiency.
    pub append_batch: Histogram,

    /// Histogram tracking the distribution of log entry counts in replication RPCs.
    ///
    /// This tracks how many log entries are included in each AppendEntries RPC
    /// sent to followers during replication, helping identify replication batch patterns.
    pub replicate_batch: Histogram,

    /// Histogram tracking the distribution of RaftMsg counts processed in a batch.
    ///
    /// This tracks how many RaftMsg are processed together before calling
    /// `run_engine_commands()`, helping identify message batching efficiency.
    pub raft_msg_per_run: Histogram,

    /// Histogram tracking the distribution of client write entries merged per batch.
    ///
    /// This tracks how many client write entries are merged together in each
    /// `RaftMsg::ClientWrite` after batching, helping identify batching efficiency.
    pub write_batch: Histogram,

    /// Histogram tracking the budget for RaftMsg processing.
    ///
    /// This tracks the maximum number of RaftMsg allowed to process in each
    /// `process_raft_msg()` call, helping understand load balancing behavior.
    pub raft_msg_budget: Histogram,

    /// Histogram tracking the budget for Notification processing.
    ///
    /// This tracks the maximum number of Notifications allowed to process in each
    /// `process_notification()` call, helping understand load balancing behavior.
    pub notification_budget: Histogram,

    /// Histogram tracking RaftMsg budget utilization in permille (0-1000).
    ///
    /// This tracks `processed * 1000 / budget` for each `process_raft_msg()` call,
    /// where 1000 means 100% utilization. Helps identify if the budget is well-tuned.
    pub raft_msg_usage_permille: Histogram,

    /// Histogram tracking Notification budget utilization in permille (0-1000).
    ///
    /// This tracks `processed * 1000 / budget` for each `process_notification()` call,
    /// where 1000 means 100% utilization. Helps identify if the budget is well-tuned.
    pub notification_usage_permille: Histogram,

    /// Count of each command type executed.
    ///
    /// This tracks how many times each command type has been executed,
    /// useful for understanding workload patterns and debugging.
    /// Indexed by `CommandName::index()`.
    pub command_counts: Vec<u64>,

    /// Count of each RaftMsg type received.
    ///
    /// This tracks how many times each RaftMsg type has been received,
    /// useful for understanding API usage patterns and debugging.
    /// Indexed by `stats::RaftMsgName::index()`.
    pub raft_msg_counts: Vec<u64>,

    /// Count of each Notification type received.
    ///
    /// This tracks how many times each Notification type has been received,
    /// useful for understanding internal message patterns and debugging.
    /// Indexed by `NotificationName::index()`.
    pub notification_counts: Vec<u64>,

    /// Per-entry timestamp tracking across 6 lifecycle stages
    /// (Proposed, Received, Submitted, Persisted, Committed, Applied).
    ///
    /// Uses a fixed-capacity ring buffer per stage.
    /// Stage-to-stage gaps reveal where latency accumulates
    /// (channel queueing, storage I/O, replication, state machine apply).
    pub log_stage: LogStages<InstantOf<C>>,

    /// Precomputed stage-to-stage duration histograms derived from [`log_stage`](Self::log_stage).
    ///
    /// Rebuilt on demand via [`build_log_stage_histograms()`](Self::build_log_stage_histograms).
    pub log_stage_histograms: LogStageHistograms,

    _phantom: PhantomData<C>,
}

impl<C> Default for RuntimeStats<C>
where C: RaftTypeConfig
{
    fn default() -> Self {
        Self::new(&Config::default())
    }
}

impl<C> RuntimeStats<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(config: &Config) -> Self {
        let _ = config;
        Self {
            apply_batch: Histogram::<()>::new(),
            append_batch: Histogram::<()>::new(),
            replicate_batch: Histogram::<()>::new(),
            raft_msg_per_run: Histogram::<()>::new(),
            write_batch: Histogram::<()>::new(),
            raft_msg_budget: Histogram::<()>::new(),
            notification_budget: Histogram::<()>::new(),
            raft_msg_usage_permille: Histogram::<()>::new(),
            notification_usage_permille: Histogram::<()>::new(),
            command_counts: vec![0; CommandName::COUNT],
            raft_msg_counts: vec![0; RaftMsgName::COUNT],
            notification_counts: vec![0; NotificationName::COUNT],
            log_stage: LogStages::new(config.log_stage_capacity(), 0),
            log_stage_histograms: LogStageHistograms::new(),
            _phantom: PhantomData,
        }
    }

    /// Record the execution of a command.
    pub fn record_command(&mut self, name: CommandName) {
        self.command_counts[name.index()] += 1;
    }

    /// Record the receipt of a RaftMsg.
    pub fn record_raft_msg(&mut self, name: RaftMsgName) {
        self.raft_msg_counts[name.index()] += 1;
    }

    /// Record the receipt of a Notification.
    pub fn record_notification(&mut self, name: NotificationName) {
        self.notification_counts[name.index()] += 1;
    }

    pub fn record_log_stage_now(&mut self, stage: Stage, index: u64) {
        #[cfg(feature = "runtime-stats")]
        self.record_log_stage(stage, index, C::now());

        #[cfg(not(feature = "runtime-stats"))]
        {
            let _ = (stage, index);
        }
    }

    #[allow(dead_code)]
    pub fn record_log_stage(&mut self, stage: Stage, index: u64, now: InstantOf<C>) {
        #[cfg(feature = "runtime-stats")]
        self.log_stage.record_stage(stage, index, now);

        #[cfg(not(feature = "runtime-stats"))]
        {
            let _ = (stage, index, now);
        }
    }

    #[allow(dead_code)]
    pub fn build_log_stage_histograms(&mut self) {
        #[cfg(feature = "runtime-stats")]
        {
            self.log_stage_histograms = self.log_stage.compute_histograms();
        }
    }

    /// Returns a displayable representation of the runtime statistics.
    ///
    /// All values are precomputed when calling this method, so the returned
    /// `RuntimeStatsDisplay` can be cheaply formatted multiple times.
    ///
    /// Use builder methods like `.human_readable()` to change the display mode.
    #[allow(dead_code)]
    pub fn display(&self) -> RuntimeStatsDisplay {
        RuntimeStatsDisplay {
            mode: DisplayMode::default(),
            apply_batch: self.apply_batch.percentile_stats(),
            append_batch: self.append_batch.percentile_stats(),
            replicate_batch: self.replicate_batch.percentile_stats(),
            raft_msg_per_run: self.raft_msg_per_run.percentile_stats(),
            write_batch: self.write_batch.percentile_stats(),
            raft_msg_budget: self.raft_msg_budget.percentile_stats(),
            notification_budget: self.notification_budget.percentile_stats(),
            raft_msg_usage_permille: self.raft_msg_usage_permille.percentile_stats(),
            notification_usage_permille: self.notification_usage_permille.percentile_stats(),
            command_counts: self.command_counts.clone(),
            raft_msg_counts: self.raft_msg_counts.clone(),
            notification_counts: self.notification_counts.clone(),
        }
    }
}
