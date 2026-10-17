use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use crate::Config;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::async_runtime::MpscSender;
use crate::async_runtime::Select3;
use crate::async_runtime::watch::WatchReceiver;
use crate::core::SharedReplicateBatch;
use crate::core::notification::Notification;
use crate::engine::quorum_loss_probe::QuorumLossProbe;
use crate::errors::ReplicationClosed;
use crate::progress::inflight_id::InflightId;
use crate::progress::stream_id::StreamId;
use crate::proposer::SendStamp;
use crate::raft_state::IOId;
use crate::replication::response::Progress;
use crate::replication::response::ReplicationResult;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::CommittedVoteOf;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::MpscSenderOf;
use crate::type_config::alias::WatchReceiverOf;
use crate::vote::raft_vote::RaftVote;

/// Shared context for replication tasks.
///
/// Contains the common state needed by both log replication (`ReplicationCore`)
/// and snapshot transmission (`SnapshotTransmitter`) tasks, including node identifiers,
/// session information, configuration, and the notification channel back to `RaftCore`.
#[derive(Clone)]
pub(crate) struct ReplicationContext<C>
where C: RaftTypeConfig
{
    /// This node id
    #[allow(dead_code)]
    pub(crate) id: C::NodeId,

    /// The ID of the target Raft node which replication events are to be sent to.
    pub(crate) target: C::NodeId,

    /// The leader this replication works for
    pub(crate) leader_vote: CommittedVoteOf<C>,

    /// Identifies which session this replication belongs to.
    pub(crate) stream_id: StreamId,

    /// The Raft's runtime config.
    pub(crate) config: Arc<Config>,

    /// A channel for sending events to the RaftCore.
    #[allow(clippy::type_complexity)]
    pub(crate) tx_notify: MpscSenderOf<C, Notification<C>>,

    /// Watch channel receiver for cancellation signal.
    ///
    /// When the sender is dropped, this signals that replication should stop.
    pub(crate) cancel_rx: WatchReceiverOf<C, ()>,

    /// Shared histogram for recording replication batch sizes.
    pub(crate) replicate_batch: SharedReplicateBatch,

    /// The quiet periods the Leader keeps after losing its quorum, published by `RaftCore`.
    pub(crate) quorum_loss_probe_rx: WatchReceiverOf<C, Option<QuorumLossProbe<InstantOf<C>>>>,
}

impl<C> ReplicationContext<C>
where C: RaftTypeConfig
{
    /// Whether the leader this replication works for has been superseded.
    ///
    /// A replication task is bound to the leader that spawned it. Once local IO is accepted under
    /// a different leader, anything this task still sends would carry a stale vote, so the caller
    /// must stop rather than finish what it started.
    pub(crate) fn leader_changed(&self, accepted_io: &IOId<C>) -> bool {
        let current_leader = accepted_io.leader_id();
        let belonging_leader = self.leader_vote.leader_id();

        if current_leader == belonging_leader {
            return false;
        }

        tracing::info!(
            "{}: Leader changed from {} to {}, quit replication",
            self,
            belonging_leader,
            current_leader
        );
        true
    }

    /// Wait until the Leader may send to the target.
    ///
    /// A Leader that has lost its quorum sends nothing in a quiet period, see
    /// [`QuorumLossProbe`]. This waits for the current one to end, or for a quorum acknowledgement
    /// that ends it early. It returns at once outside a quiet period.
    pub(crate) async fn wait_for_send_period(&mut self) -> Result<(), ReplicationClosed> {
        loop {
            let probe = *self.quorum_loss_probe_rx.borrow_and_update();
            let Some(quiet_for) = probe.and_then(|probe| probe.quiet_for(C::now())) else {
                return Ok(());
            };

            tracing::debug!("{}: quorum lost, keep quiet for {:?}", self, quiet_for);

            self.wait_for_probe_change(Some(quiet_for)).await?;
        }
    }

    /// Wait for a quiet period to interrupt an active snapshot, following quorum acknowledgements.
    pub(crate) async fn wait_for_quiet_period(&mut self) -> Result<(), ReplicationClosed> {
        loop {
            let probe = *self.quorum_loss_probe_rx.borrow_and_update();
            let remaining = match probe {
                Some(probe) => {
                    let now = C::now();
                    let (is_quiet, remaining) = probe.phase(now);
                    if is_quiet {
                        return Ok(());
                    }
                    Some(remaining)
                }
                None => None,
            };
            self.wait_for_probe_change(remaining).await?;
        }
    }

    async fn wait_for_probe_change(&mut self, timeout: Option<Duration>) -> Result<(), ReplicationClosed> {
        let sleep = async move {
            match timeout {
                Some(duration) => C::sleep(duration).await,
                None => std::future::pending::<()>().await,
            }
        };
        let cancel = self.cancel_rx.changed();
        let probe_changed = self.quorum_loss_probe_rx.changed();
        match C::select3(cancel, probe_changed, sleep).await {
            Select3::First(_) => Err(ReplicationClosed::new("canceled")),
            Select3::Second(Err(_)) => Err(ReplicationClosed::new("RaftCore is dropped")),
            Select3::Second(Ok(())) | Select3::Third(()) => Ok(()),
        }
    }

    /// Report a storage failure to [`RaftCore`].
    ///
    /// [`RaftCore`]: crate::core::RaftCore
    pub(crate) async fn notify_storage_error(&self, error: StorageError<C>) {
        self.tx_notify.send(Notification::StorageError { error }).await.ok();
    }

    /// Report that the target answered a request sent at `sending_time`.
    ///
    /// Any successful exchange with the target also proves the leader is still reachable, so this
    /// doubles as a heartbeat acknowledgement. A replication request belongs to no heartbeat round,
    /// so its stamp has round `0`.
    pub(crate) async fn notify_heartbeat_progress(&self, sending_time: InstantOf<C>) {
        self.tx_notify
            .send(Notification::HeartbeatProgress {
                stream_id: self.stream_id,
                target: self.target.clone(),
                sending_time: SendStamp::new(sending_time, 0),
            })
            .await
            .ok();
    }

    /// Report the outcome of the payload identified by `inflight_id`.
    ///
    /// `inflight_id` is `None` when nothing was sent that [`RaftCore`] is waiting on, such as a
    /// bare committed-index update.
    ///
    /// [`RaftCore`]: crate::core::RaftCore
    pub(crate) async fn notify_progress(
        &self,
        result: Result<ReplicationResult<C>, String>,
        inflight_id: Option<InflightId>,
    ) {
        self.tx_notify
            .send(Notification::ReplicationProgress {
                stream_id: self.stream_id,
                progress: Progress {
                    target: self.target.clone(),
                    result,
                },
                inflight_id,
            })
            .await
            .ok();
    }
}

impl<C> fmt::Display for ReplicationContext<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{id: {}, target: {}, {}}}", self.id, self.target, self.stream_id)
    }
}

impl<C> fmt::Debug for ReplicationContext<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReplicationContext")
            .field("id", &self.id)
            .field("target", &self.target)
            .field("session_id", &self.stream_id)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}
