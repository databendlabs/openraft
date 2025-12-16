use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use futures::StreamExt;

use crate::Config;
use crate::RaftTypeConfig;
use crate::async_runtime::watch::WatchReceiver;
use crate::core::heartbeat::errors::RaftCoreClosed;
use crate::core::heartbeat::errors::Stopped;
use crate::core::heartbeat::event::HeartbeatEvent;
use crate::core::notification::Notification;
use crate::network::RPCOption;
use crate::network::v2::RaftNetworkV2;
use crate::progress::stream_id::StreamId;
use crate::raft::AppendEntriesRequest;
use crate::raft::StreamAppendError;
use crate::raft::StreamAppendResult;
use crate::replication::Progress;
use crate::replication::response::ReplicationResult;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::MpscSenderOf;
use crate::type_config::alias::OneshotReceiverOf;
use crate::type_config::alias::WatchReceiverOf;
use crate::type_config::async_runtime::mpsc::MpscSender;
use crate::vote::committed::CommittedVote;

/// A dedicated worker sending heartbeat to a specific follower.
pub struct HeartbeatWorker<C, N>
where
    C: RaftTypeConfig,
    N: RaftNetworkV2<C>,
{
    pub(crate) id: C::NodeId,

    /// The leader this heartbeat worker works for
    pub(crate) leader_vote: CommittedVote<C>,

    /// A unique stream.
    pub(crate) stream_id: StreamId,

    /// The receiver will be changed when a new heartbeat is needed to be sent.
    pub(crate) rx: WatchReceiverOf<C, Option<HeartbeatEvent<C>>>,

    pub(crate) network: N,

    pub(crate) target: C::NodeId,

    #[allow(dead_code)]
    pub(crate) node: C::Node,

    pub(crate) config: Arc<Config>,

    /// For sending back result to the [`RaftCore`].
    ///
    /// [`RaftCore`]: crate::core::RaftCore
    pub(crate) tx_notification: MpscSenderOf<C, Notification<C>>,
}

impl<C, N> fmt::Display for HeartbeatWorker<C, N>
where
    C: RaftTypeConfig,
    N: RaftNetworkV2<C>,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HeartbeatWorker(id={}, target={})", self.id, self.target)
    }
}

impl<C, N> HeartbeatWorker<C, N>
where
    C: RaftTypeConfig,
    N: RaftNetworkV2<C>,
{
    pub(crate) async fn run(self, rx_shutdown: OneshotReceiverOf<C, ()>) {
        let res = self.do_run(rx_shutdown).await;
        tracing::info!("HeartbeatWorker finished with result: {:?}", res);
    }

    pub(crate) async fn do_run(mut self, mut rx_shutdown: OneshotReceiverOf<C, ()>) -> Result<(), Stopped> {
        loop {
            tracing::debug!("{} is waiting for a new heartbeat event.", self);

            futures::select! {
                _ = (&mut rx_shutdown).fuse() => {
                    tracing::info!("{} is shutdown.", self);
                    return Err(Stopped::ReceivedShutdown);
                },
                _ = self.rx.changed().fuse() => {},
            }

            let heartbeat: Option<HeartbeatEvent<C>> = self.rx.borrow_watched().clone();

            // None is the initial value of the WatchReceiver, ignore it.
            let Some(heartbeat) = heartbeat else {
                continue;
            };

            let timeout = Duration::from_millis(self.config.heartbeat_interval);
            let option = RPCOption::new(timeout);

            let payload = AppendEntriesRequest {
                vote: heartbeat.session_id.leader_vote.clone().into_vote(),
                // Use last known matching log id as prev_log_id to detect follower state reversion.
                // prev_log_id == None does not conflict.
                //
                // Fail test `t99_issue_1500_heartbeat_cause_reversion_panic` by changing the
                // following line to `prev_log_id = heartbeat.committed.clone()`.
                prev_log_id: heartbeat.matching.clone(),
                leader_commit: heartbeat.committed.clone(),
                entries: vec![],
            };

            let input_stream = Box::pin(futures::stream::once(async { payload }));

            let res = C::timeout(timeout, async {
                let mut output = self.network.stream_append(input_stream, option).await?;
                output.next().await.transpose()
            })
            .await;

            tracing::debug!("{} sent a heartbeat: {}, result: {:?}", self, heartbeat, res);

            match res {
                Ok(Ok(Some(stream_result))) => {
                    self.handle_stream_result(stream_result, &heartbeat).await?;
                }
                Ok(Ok(None)) => {
                    // Stream returned no response - treat as network error
                    tracing::warn!("{} heartbeat stream returned no response", self);
                }
                _ => {
                    tracing::warn!("{} failed to send a heartbeat: {:?}", self, res);
                }
            };
        }
    }

    /// Handle the stream append result, send appropriate notifications.
    async fn handle_stream_result(
        &self,
        result: StreamAppendResult<C>,
        heartbeat: &HeartbeatEvent<C>,
    ) -> Result<(), RaftCoreClosed> {
        match result {
            Ok(_) => {
                self.send_heartbeat_progress(heartbeat).await?;
            }
            Err(StreamAppendError::HigherVote(vote)) => {
                tracing::debug!(
                    "seen a higher vote({vote}) from {}; when:(sending heartbeat)",
                    self.target
                );

                let noti = Notification::HigherVote {
                    target: self.target.clone(),
                    higher: vote,
                    leader_vote: self.leader_vote.clone(),
                };

                self.send_notification(noti, "Seeing higher Vote").await?;
                // Higher vote means leadership is not granted, don't send HeartbeatProgress
            }
            Err(StreamAppendError::Conflict(_conflict_log_id)) => {
                // The follower does not have `matching` log id.
                // Use `matching` (which may be None) as the conflict point.
                //
                // Safe unwrap(): a None never conflict
                let conflict_log_id = heartbeat.matching.clone().unwrap();

                let noti = Notification::ReplicationProgress {
                    progress: Progress {
                        target: self.target.clone(),
                        result: Ok(ReplicationResult(Err(conflict_log_id))),
                    },
                    inflight_id: None,
                };

                self.send_notification(noti, "Seeing conflict").await?;
                self.send_heartbeat_progress(heartbeat).await?;
            }
        }
        Ok(())
    }

    async fn send_heartbeat_progress(&self, heartbeat: &HeartbeatEvent<C>) -> Result<(), RaftCoreClosed> {
        let noti = Notification::HeartbeatProgress {
            stream_id: self.stream_id,
            sending_time: heartbeat.time,
            target: self.target.clone(),
        };
        self.send_notification(noti, "send HeartbeatProgress").await
    }

    async fn send_notification(
        &self,
        notification: Notification<C>,
        when: impl fmt::Display,
    ) -> Result<(), RaftCoreClosed> {
        let res = self.tx_notification.send(notification).await;

        if let Err(e) = res {
            let notification = e.0;
            tracing::error!("{self} failed to send {notification} to RaftCore; when:({when})");
            return Err(RaftCoreClosed);
        }
        Ok(())
    }
}
