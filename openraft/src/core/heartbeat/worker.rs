use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::StreamExt;

use crate::Config;
use crate::RaftTypeConfig;
use crate::async_runtime::watch::WatchReceiver;
use crate::core::heartbeat::errors::RaftCoreClosed;
use crate::core::heartbeat::errors::Stopped;
use crate::core::heartbeat::event::HeartbeatEvent;
use crate::core::notification::Notification;
use crate::network::NetStreamAppend;
use crate::network::RPCOption;
use crate::progress::stream_id::StreamId;
use crate::raft::AppendEntriesRequest;
use crate::raft::StreamAppendError;
use crate::raft::StreamAppendResult;
use crate::replication::Progress;
use crate::replication::response::ReplicationResult;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::CommittedVoteOf;
use crate::type_config::alias::MpscSenderOf;
use crate::type_config::alias::OneshotReceiverOf;
use crate::type_config::alias::WatchReceiverOf;
use crate::type_config::async_runtime::mpsc::MpscSender;

/// A dedicated worker sending heartbeat to a specific follower.
pub struct HeartbeatWorker<C, N>
where
    C: RaftTypeConfig,
    N: NetStreamAppend<C>,
{
    pub(crate) id: C::NodeId,

    /// The leader this heartbeat worker works for
    pub(crate) leader_vote: CommittedVoteOf<C>,

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
    N: NetStreamAppend<C>,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HeartbeatWorker(id={}, target={})", self.id, self.target)
    }
}

impl<C, N> HeartbeatWorker<C, N>
where
    C: RaftTypeConfig,
    N: NetStreamAppend<C>,
{
    pub(crate) async fn run(self, rx_shutdown: OneshotReceiverOf<C, ()>) {
        let res = self.do_run(rx_shutdown).await;
        tracing::info!("HeartbeatWorker finished with result: {:?}", res);
    }

    pub(crate) async fn do_run(mut self, mut rx_shutdown: OneshotReceiverOf<C, ()>) -> Result<(), Stopped> {
        loop {
            tracing::debug!("{} is waiting for a new heartbeat event.", self);

            futures_util::select! {
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
                vote: self.leader_vote.clone().into_vote(),
                // Use last known matching log id as prev_log_id to detect follower state reversion.
                // prev_log_id == None does not conflict.
                //
                // Fail test `t99_issue_1500_heartbeat_cause_reversion_panic` by changing the
                // following line to `prev_log_id = heartbeat.cluster_committed.clone()`.
                prev_log_id: heartbeat.matching.clone(),
                leader_commit: heartbeat.cluster_committed.clone(),
                entries: vec![],
            };

            let input_stream = Box::pin(futures_util::stream::once(async { payload }));

            let res = C::timeout(timeout, async {
                let mut output = self.network.stream_append(input_stream, option).await?.fuse();
                let res = output.next().await.transpose();
                // Poll output until it returns `None`, which should be the very next value, to
                // allow the network layer to close the connection cleanly.
                let extra: Vec<_> = output.collect().await;
                if !extra.is_empty() {
                    tracing::warn!("{} unexpected extra heartbeat responses: {:?}", self, extra);
                }
                res
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
                    stream_id: self.stream_id,
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::Ordering;
    use std::task::Poll;

    use futures_util::Stream;
    use futures_util::StreamExt;

    use super::HeartbeatWorker;
    use crate::Config;
    use crate::OptionalSend;
    use crate::async_runtime::MpscReceiver;
    use crate::async_runtime::watch::WatchSender;
    use crate::base::BoxFuture;
    use crate::base::BoxStream;
    use crate::core::heartbeat::event::HeartbeatEvent;
    use crate::core::notification::Notification;
    use crate::engine::testing::UTConfig;
    use crate::engine::testing::UTLeaderId;
    use crate::error::RPCError;
    use crate::impls::Vote;
    use crate::network::NetStreamAppend;
    use crate::network::RPCOption;
    use crate::progress::stream_id::StreamId;
    use crate::raft::AppendEntriesRequest;
    use crate::raft::StreamAppendResult;
    use crate::type_config::TypeConfigExt;
    use crate::vote::raft_vote::RaftVoteExt;

    type C = UTConfig;

    /// A network whose response stream yields one success per request and ends when the input
    /// ends.
    ///
    /// `drained` is set when the response stream observes the end of the input.
    struct OneResponsePerRequestNetwork {
        drained: Arc<AtomicBool>,
    }

    impl NetStreamAppend<C> for OneResponsePerRequestNetwork {
        fn stream_append<'s, S>(
            &'s mut self,
            mut input: S,
            _option: RPCOption,
        ) -> BoxFuture<'s, Result<BoxStream<'s, Result<StreamAppendResult<C>, RPCError<C>>>, RPCError<C>>>
        where
            S: Stream<Item = AppendEntriesRequest<C>> + OptionalSend + Unpin + 'static,
        {
            let drained = self.drained.clone();
            let output = futures_util::stream::poll_fn(move |cx| match input.poll_next_unpin(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(Some(_request)) => Poll::Ready(Some(Ok(Ok(None)))),
                Poll::Ready(None) => {
                    drained.store(true, Ordering::SeqCst);
                    Poll::Ready(None)
                }
            });

            let output: BoxStream<'s, _> = Box::pin(output);
            Box::pin(async move { Ok(output) })
        }
    }

    #[test]
    fn test_heartbeat_polls_response_stream_to_end() {
        C::run(async {
            let drained = Arc::new(AtomicBool::new(false));
            let (tx_notification, mut rx_notification) = C::mpsc(16);
            let (_tx_shutdown, rx_shutdown) = C::oneshot();

            tracing::info!(target = 2, "--- spawn a heartbeat worker");
            let tx_event = {
                let (tx_event, rx_event) = C::watch_channel(None);

                let worker = HeartbeatWorker::<C, _> {
                    id: 1,
                    leader_vote: Vote::<UTLeaderId>::new(1, 1).to_committed(),
                    stream_id: StreamId::new(1),
                    rx: rx_event,
                    network: OneResponsePerRequestNetwork {
                        drained: drained.clone(),
                    },
                    target: 2,
                    node: (),
                    config: Arc::new(Config::default()),
                    tx_notification,
                };
                C::spawn(worker.do_run(rx_shutdown));

                tx_event
            };

            tracing::info!("--- send one heartbeat and wait for the worker to report its progress");
            {
                let heartbeat = HeartbeatEvent {
                    time: C::now(),
                    matching: None,
                    cluster_committed: None,
                };
                tx_event.send(Some(heartbeat)).unwrap();

                let notification = rx_notification.recv().await.unwrap();
                assert!(matches!(notification, Notification::HeartbeatProgress { .. }));
            }

            tracing::info!("--- the response stream must be drained before the progress is reported");
            assert!(drained.load(Ordering::SeqCst));
        });
    }
}
