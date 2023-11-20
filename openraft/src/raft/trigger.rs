//! Trigger an action to RaftCore by external caller.

use crate::core::raft_msg::external_command::ExternalCommand;
use crate::error::Fatal;
use crate::raft::RaftInner;
use crate::RaftTypeConfig;

/// Trigger is an interface to trigger an action to RaftCore by external caller.
pub struct Trigger<'r, C>
where C: RaftTypeConfig
{
    raft_inner: &'r RaftInner<C>,
}

impl<'r, C> Trigger<'r, C>
where C: RaftTypeConfig
{
    pub(in crate::raft) fn new(raft_inner: &'r RaftInner<C>) -> Self {
        Self { raft_inner }
    }

    /// Trigger election at once and return at once.
    ///
    /// Returns error when RaftCore has [`Fatal`] error, e.g. shut down or having storage error.
    /// It is not affected by `Raft::enable_elect(false)`.
    pub async fn elect(&self) -> Result<(), Fatal<C::NodeId>> {
        self.raft_inner.send_external_command(ExternalCommand::Elect, "trigger_elect").await
    }

    /// Trigger a heartbeat at once and return at once.
    ///
    /// Returns error when RaftCore has [`Fatal`] error, e.g. shut down or having storage error.
    /// It is not affected by `Raft::enable_heartbeat(false)`.
    pub async fn heartbeat(&self) -> Result<(), Fatal<C::NodeId>> {
        self.raft_inner.send_external_command(ExternalCommand::Heartbeat, "trigger_heartbeat").await
    }

    /// Trigger to build a snapshot at once and return at once.
    ///
    /// Returns error when RaftCore has [`Fatal`] error, e.g. shut down or having storage error.
    pub async fn snapshot(&self) -> Result<(), Fatal<C::NodeId>> {
        self.raft_inner.send_external_command(ExternalCommand::Snapshot, "trigger_snapshot").await
    }

    /// Initiate the log purge up to and including the given `upto` log index.
    ///
    /// Logs that are not included in a snapshot will **NOT** be purged.
    /// In such scenario it will delete as many log as possible.
    /// The [`max_in_snapshot_log_to_keep`] config is not taken into account
    /// when purging logs.
    ///
    /// It returns error only when RaftCore has [`Fatal`] error, e.g. shut down or having storage
    /// error.
    ///
    /// Openraft won't purge logs at once, e.g. it may be delayed by several seconds, because if it
    /// is a leader and a replication task has been replicating the logs to a follower, the logs
    /// can't be purged until the replication task is finished.
    ///
    /// [`max_in_snapshot_log_to_keep`]: `crate::Config::max_in_snapshot_log_to_keep`
    pub async fn purge_log(&self, upto: u64) -> Result<(), Fatal<C::NodeId>> {
        self.raft_inner.send_external_command(ExternalCommand::PurgeLog { upto }, "purge_log").await
    }
}
