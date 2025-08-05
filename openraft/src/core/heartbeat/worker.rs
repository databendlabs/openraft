use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;

use crate::async_runtime::watch::WatchReceiver;
use crate::async_runtime::MpscUnboundedSender;
use crate::core::heartbeat::errors::RaftCoreClosed;
use crate::core::heartbeat::errors::Stopped;
use crate::core::heartbeat::event::HeartbeatEvent;
use crate::core::notification::Notification;
use crate::network::v2::RaftNetworkV2;
use crate::network::RPCOption;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponse;
use crate::replication::response::ReplicationResult;
use crate::replication::Progress;
use crate::type_config::alias::MpscUnboundedSenderOf;
use crate::type_config::alias::OneshotReceiverOf;
use crate::type_config::alias::WatchReceiverOf;
use crate::type_config::TypeConfigExt;
use crate::Config;
use crate::RaftTypeConfig;

/// A dedicated worker sending heartbeat to a specific follower.
pub struct HeartbeatWorker<C, N>
where
    C: RaftTypeConfig,
    N: RaftNetworkV2<C>,
{
    pub(crate) id: C::NodeId,

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
    pub(crate) tx_notification: MpscUnboundedSenderOf<C, Notification<C>>,
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
                // Use committed log id as prev_log_id to detect follower state reversion.
                // prev_log_id == None does not conflict.
                prev_log_id: heartbeat.committed.clone(),
                leader_commit: heartbeat.committed.clone(),
                entries: vec![],
            };

            let res = C::timeout(timeout, self.network.append_entries(payload, option)).await;
            tracing::debug!("{} sent a heartbeat: {}, result: {:?}", self, heartbeat, res);

            match res {
                Ok(Ok(x)) => {
                    let response: AppendEntriesResponse<C> = x;

                    match response {
                        AppendEntriesResponse::Success => {}
                        AppendEntriesResponse::PartialSuccess(_matching) => {}
                        AppendEntriesResponse::HigherVote(vote) => {
                            tracing::debug!(
                                "seen a higher vote({vote}) from {}; when:(sending heartbeat)",
                                self.target
                            );

                            let noti = Notification::HigherVote {
                                target: self.target.clone(),
                                higher: vote,
                                leader_vote: heartbeat.session_id.committed_vote(),
                            };

                            self.send_notification(noti, "Seeing higher Vote")?;
                        }
                        AppendEntriesResponse::Conflict => {
                            let conflict = heartbeat.committed.unwrap();

                            let noti = Notification::ReplicationProgress {
                                has_payload: false,
                                progress: Progress {
                                    session_id: heartbeat.session_id.clone(),
                                    target: self.target.clone(),
                                    result: Ok(ReplicationResult(Err(conflict))),
                                },
                            };

                            self.send_notification(noti, "Seeing conflict")?;
                        }
                    }

                    let noti = Notification::HeartbeatProgress {
                        session_id: heartbeat.session_id.clone(),
                        sending_time: heartbeat.time,
                        target: self.target.clone(),
                    };

                    self.send_notification(noti, "send HeartbeatProgress")?;
                }
                _ => {
                    tracing::warn!("{} failed to send a heartbeat: {:?}", self, res);
                }
            }
        }
    }

    fn send_notification(&self, notification: Notification<C>, when: impl fmt::Display) -> Result<(), RaftCoreClosed> {
        let res = self.tx_notification.send(notification);

        if let Err(e) = res {
            let notification = e.0;
            tracing::error!("{self} failed to send {notification} to RaftCore; when:({when})");
            return Err(RaftCoreClosed);
        }
        Ok(())
    }
}
