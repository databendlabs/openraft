//! Trigger an action to RaftCore by an external caller.

use crate::RaftTypeConfig;
use crate::core::raft_msg::external_command::ExternalCommand;
use crate::error::AllowNextRevertError;
use crate::error::Fatal;
use crate::raft::RaftInner;
use crate::type_config::TypeConfigExt;

/// Trigger is an interface to trigger an action to RaftCore by external caller.
///
/// It is created with [`Raft::trigger()`].
///
/// For example, to trigger an election at once, you can use the following code
/// ```ignore
/// raft.trigger().elect().await?;
/// ```
///
/// Or to fire a heartbeat, building a snapshot, or purging logs:
///
/// ```ignore
/// raft.trigger().heartbeat().await?;
/// raft.trigger().snapshot().await?;
/// raft.trigger().purge_log().await?;
/// ```
///
/// [`Raft::trigger()`]: crate::Raft::trigger
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
    /// Returns error when RaftCore has [`Fatal`] error, e.g., shut down or having storage error.
    /// It is not affected by `Raft::enable_elect(false)`.
    pub async fn elect(&self) -> Result<(), Fatal<C>> {
        self.raft_inner.send_external_command(ExternalCommand::Elect).await
    }

    /// Trigger a heartbeat at once and return at once.
    ///
    /// Returns error when RaftCore has [`Fatal`] error, e.g., shut down or having storage error.
    /// It is not affected by `Raft::enable_heartbeat(false)`.
    pub async fn heartbeat(&self) -> Result<(), Fatal<C>> {
        self.raft_inner.send_external_command(ExternalCommand::Heartbeat).await
    }

    /// Trigger to build a snapshot at once and return at once.
    ///
    /// Returns error when RaftCore has [`Fatal`] error, e.g., shut down or having storage error.
    pub async fn snapshot(&self) -> Result<(), Fatal<C>> {
        self.raft_inner.send_external_command(ExternalCommand::Snapshot).await
    }

    /// Initiate the log purge up to and including the given `upto` log index.
    ///
    /// Logs that are not included in a snapshot will **NOT** be purged.
    /// In such a scenario it will delete as many logs as possible.
    /// The [`max_in_snapshot_log_to_keep`] config is not taken into account
    /// when purging logs.
    ///
    /// It returns an error only when RaftCore has [`Fatal`] error, e.g., shut down or having
    /// storage error.
    ///
    /// Openraft won't purge logs at once, e.g., it may be delayed by several seconds, because if it
    /// is a leader and a replication task has been replicating the logs to a follower, the logs
    /// can't be purged until the replication task is finished.
    ///
    /// [`max_in_snapshot_log_to_keep`]: `crate::Config::max_in_snapshot_log_to_keep`
    pub async fn purge_log(&self, upto: u64) -> Result<(), Fatal<C>> {
        self.raft_inner.send_external_command(ExternalCommand::PurgeLog { upto }).await
    }

    /// Submit a command to inform RaftCore to transfer leadership to the specified node.
    ///
    /// If this node is not a Leader, it is just ignored.
    pub async fn transfer_leader(&self, to: C::NodeId) -> Result<(), Fatal<C>> {
        self.raft_inner.send_external_command(ExternalCommand::TriggerTransferLeader { to }).await
    }

    /// Request the RaftCore to allow to reset replication for a specific node when log revert is
    /// detected.
    ///
    /// - `allow=true`: This method instructs the RaftCore to allow the target node's log to revert
    ///   to a previous state for one time.
    /// - `allow=false`: This method instructs the RaftCore to panic if the target node's log revert
    ///
    /// This method returns a [`Fatal`] error if it failed to send the request to RaftCore, e.g.,
    /// when RaftCore is shut down.
    /// Otherwise, it returns an `Ok(Result<_,_>)`, the inner result is:
    /// - `Ok(())` if the request is successfully processed,
    /// - or `Err(AllowNextRevertError)` explaining why the request is rejected.
    ///
    /// ### Behavior
    ///
    /// - If this node is the Leader, it will attempt to replicate logs to the target node from the
    ///   beginning.
    /// - If this node is not the Leader, the request is ignored.
    /// - If the target node is not found, the request is ignored.
    ///
    /// ### Automatic Replication Reset
    ///
    /// When the [`Config::allow_log_reversion`] is enabled, the Leader automatically
    /// resets replication if it detects that the target node's log has reverted. This
    /// feature is primarily useful in testing environments.
    ///
    /// ### Production Considerations
    ///
    /// In production environments, state reversion is a critical issue that should not be
    /// automatically handled. However, there may be scenarios where a Follower's data is
    /// intentionally removed and needs to rejoin the cluster(without membership changes). In such
    /// cases, the Leader should reinitialize replication for that node with the following steps:
    /// - Shut down the target node.
    /// - call [`Self::allow_next_revert`] on the Leader.
    /// - Clear the target node's data directory.
    /// - Restart the target node.
    ///
    /// [`Config::allow_log_reversion`]: `crate::Config::allow_log_reversion`
    pub async fn allow_next_revert(
        &self,
        to: &C::NodeId,
        allow: bool,
    ) -> Result<Result<(), AllowNextRevertError<C>>, Fatal<C>> {
        let (tx, rx) = C::oneshot();
        self.raft_inner
            .send_external_command(ExternalCommand::AllowNextRevert {
                to: to.clone(),
                allow,
                tx,
            })
            .await?;

        let res: Result<(), AllowNextRevertError<C>> = self.raft_inner.recv_msg(rx).await?;

        Ok(res)
    }
}
