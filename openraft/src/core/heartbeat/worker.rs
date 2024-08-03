use std::fmt;
use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;

use crate::async_runtime::watch::WatchReceiver;
use crate::async_runtime::MpscUnboundedSender;
use crate::core::heartbeat::event::HeartbeatEvent;
use crate::core::notification::Notification;
use crate::network::v2::RaftNetworkV2;
use crate::network::RPCOption;
use crate::raft::AppendEntriesRequest;
use crate::type_config::alias::MpscUnboundedSenderOf;
use crate::type_config::alias::OneshotReceiverOf;
use crate::type_config::alias::WatchReceiverOf;
use crate::type_config::TypeConfigExt;
use crate::Config;
use crate::RaftTypeConfig;

/// A dedicate worker sending heartbeat to a specific follower.
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
    pub(crate) async fn run(mut self, mut rx_shutdown: OneshotReceiverOf<C, ()>) {
        loop {
            tracing::debug!("{} is waiting for a new heartbeat event.", self);

            futures::select! {
                _ = (&mut rx_shutdown).fuse() => {
                    tracing::info!("{} is shutdown.", self);
                    return;
                },
                _ = self.rx.changed().fuse() => {},
            }

            let heartbeat = *self.rx.borrow_watched();

            // None is the initial value of the WatchReceiver, ignore it.
            let Some(heartbeat) = heartbeat else {
                continue;
            };

            let timeout = Duration::from_millis(self.config.heartbeat_interval);
            let option = RPCOption::new(timeout);

            let payload = AppendEntriesRequest {
                vote: *heartbeat.leader_vote.deref(),
                prev_log_id: None,
                leader_commit: heartbeat.committed,
                entries: vec![],
            };

            let res = C::timeout(timeout, self.network.append_entries(payload, option)).await;
            tracing::debug!("{} sent a heartbeat: {}, result: {:?}", self, heartbeat, res);

            match res {
                Ok(Ok(_)) => {
                    let res = self.tx_notification.send(Notification::HeartbeatProgress {
                        leader_vote: heartbeat.leader_vote,
                        sending_time: heartbeat.time,
                        target: self.target,
                    });

                    if res.is_err() {
                        tracing::error!("{} failed to send a heartbeat progress to RaftCore. quit", self);
                        return;
                    }
                }
                _ => {
                    tracing::warn!("{} failed to send a heartbeat: {:?}", self, res);
                }
            }
        }
    }
}
