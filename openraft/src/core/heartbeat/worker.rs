use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::StreamExt;

use crate::Config;
use crate::OptionalSend;
use crate::RaftTypeConfig;
use crate::async_runtime::watch::WatchReceiver;
use crate::core::heartbeat::errors::RaftCoreClosed;
use crate::core::heartbeat::event::HeartbeatEvent;
use crate::core::notification::Notification;
use crate::network::NetStreamAppend;
use crate::network::RPCOption;
use crate::progress::stream_id::StreamId;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendError;
use crate::raft::AppendResult;
use crate::replication::Progress;
use crate::replication::response::ReplicationResult;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::CancelHandleOf;
use crate::type_config::alias::CommittedVoteOf;
use crate::type_config::alias::MpscSenderOf;
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
    pub(crate) fn run(self) -> (CancelHandleOf<C>, impl Future<Output = ()> + OptionalSend) {
        let (cancel, future) = C::cancellable(self.do_run());
        // An `async` block awaiting `future` would reserve its bytes twice: once for the captured
        // variable and once for the moved value it polls (rust-lang/rust#62958). `map()` stores
        // it once.
        let task = future.map(|res| {
            tracing::info!("HeartbeatWorker finished with result: {:?}", res);
        });
        (cancel, task)
    }

    async fn do_run(mut self) -> Result<(), RaftCoreClosed> {
        loop {
            tracing::debug!("{} is waiting for a new heartbeat event.", self);

            let changed = self.rx.changed().await;
            if changed.is_err() {
                return Ok(());
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
                // allow the network layer to close the connection cleanly. Save the first extra
                // response for logging.
                let extra = output.next().await;
                output.for_each(|_| async {}).await;
                if let Some(extra) = extra {
                    tracing::warn!("{} unexpected extra heartbeat response: {:?}", self, extra);
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
        result: AppendResult<C>,
        heartbeat: &HeartbeatEvent<C>,
    ) -> Result<(), RaftCoreClosed> {
        match result {
            Ok(_) => {
                self.send_heartbeat_progress(heartbeat).await?;
            }
            Err(AppendError::HigherVote(vote)) => {
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
            Err(AppendError::Conflict { .. }) => {
                // The follower does not have `matching` log id.
                // Use `matching` (which may be None) as the conflict point.
                //
                // Safe unwrap(): a None never conflict
                let conflict_log_id = heartbeat.matching.clone().unwrap();

                // Drop the conflict hint: a heartbeat has no inflight id, so RaftCore cannot tell
                // whether the hint answers its current request, and ignores it.
                let noti = Notification::ReplicationProgress {
                    stream_id: self.stream_id,
                    progress: Progress {
                        target: self.target.clone(),
                        result: Ok(ReplicationResult(Err(conflict_log_id))),
                        hint: None,
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
    use std::time::Duration;

    use futures_util::FutureExt;
    use futures_util::Stream;
    use futures_util::StreamExt;

    use super::HeartbeatWorker;
    use crate::Config;
    use crate::OptionalSend;
    use crate::async_runtime::MpscReceiver;
    use crate::async_runtime::MpscSender;
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
    use crate::proposer::SendStamp;
    use crate::raft::AppendEntriesRequest;
    use crate::raft::AppendResult;
    use crate::type_config::TypeConfigExt;
    use crate::type_config::alias::MpscSenderOf;
    use crate::type_config::alias::WatchSenderOf;
    use crate::vote::raft_vote::RaftVoteExt;

    type C = UTConfig;

    const TEST_TIMEOUT: Duration = Duration::from_secs(1);
    const BLOCKED_HEARTBEAT_MS: u64 = 3_600_000;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum BlockAt {
        Idle,
        Setup,
        Response,
        Notification,
    }

    struct TestNetwork {
        block_at: BlockAt,
        reached: Arc<AtomicBool>,
        dropped: Arc<AtomicBool>,
    }

    impl Drop for TestNetwork {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::Relaxed);
        }
    }

    impl NetStreamAppend<C> for TestNetwork {
        fn stream_append<'s, S>(
            &'s mut self,
            mut input: S,
            _option: RPCOption,
        ) -> BoxFuture<'s, Result<BoxStream<'s, Result<AppendResult<C>, RPCError<C>>>, RPCError<C>>>
        where
            S: Stream<Item = AppendEntriesRequest<C>> + OptionalSend + Unpin + 'static,
        {
            if self.block_at == BlockAt::Setup {
                self.reached.store(true, Ordering::Relaxed);
                return Box::pin(std::future::pending());
            }

            let block_at = self.block_at;
            let reached = self.reached.clone();
            let output = futures_util::stream::poll_fn(move |cx| {
                if block_at == BlockAt::Response {
                    reached.store(true, Ordering::Relaxed);
                    return Poll::Pending;
                }
                match input.poll_next_unpin(cx) {
                    Poll::Pending => Poll::Pending,
                    Poll::Ready(Some(_request)) => Poll::Ready(Some(Ok(Ok(None)))),
                    Poll::Ready(None) => {
                        reached.store(true, Ordering::Relaxed);
                        Poll::Ready(None)
                    }
                }
            });
            let output: BoxStream<'s, _> = Box::pin(output);
            Box::pin(async move { Ok(output) })
        }
    }

    struct Fixture {
        worker: HeartbeatWorker<C, TestNetwork>,
        event_tx: WatchSenderOf<C, Option<HeartbeatEvent<C>>>,
        reached: Arc<AtomicBool>,
        dropped: Arc<AtomicBool>,
    }

    impl Fixture {
        fn new(block_at: BlockAt, tx_notification: MpscSenderOf<C, Notification<C>>) -> Self {
            let reached = Arc::new(AtomicBool::new(false));
            let dropped = Arc::new(AtomicBool::new(false));
            let (event_tx, rx) = C::watch_channel(None);
            let vote = Vote::<UTLeaderId>::new(1, 1);
            let leader_vote = vote.to_committed();
            let network = TestNetwork {
                block_at,
                reached: reached.clone(),
                dropped: dropped.clone(),
            };
            let config = Config {
                heartbeat_interval: BLOCKED_HEARTBEAT_MS,
                ..Config::default()
            };
            Self {
                worker: HeartbeatWorker {
                    id: 1,
                    leader_vote,
                    stream_id: StreamId::new(1),
                    rx,
                    network,
                    target: 2,
                    node: (),
                    config: Arc::new(config),
                    tx_notification,
                },
                event_tx,
                reached,
                dropped,
            }
        }
    }

    #[test]
    fn test_heartbeat_polls_response_stream_to_end() {
        C::run(async {
            let (tx_notification, mut rx_notification) = C::mpsc(1);
            let fixture = Fixture::new(BlockAt::Notification, tx_notification);
            let heartbeat = HeartbeatEvent {
                time: SendStamp::new(C::now(), 1),
                matching: None,
                cluster_committed: None,
            };
            fixture.event_tx.send(Some(heartbeat)).unwrap();
            let (_cancel, future) = fixture.worker.run();
            let join = C::spawn(future);
            let notification = C::timeout(TEST_TIMEOUT, rx_notification.recv()).await.unwrap();
            let progress = matches!(notification, Some(Notification::HeartbeatProgress { .. }));
            assert!(progress);
            let drained = fixture.reached.load(Ordering::Relaxed);
            assert!(drained, "the response stream must drain before progress is reported");
            drop(fixture.event_tx);
            C::timeout(TEST_TIMEOUT, join).await.unwrap().unwrap();
        });
    }

    #[test]
    fn cancellation_interrupts_every_worker_wait() {
        C::run(async {
            for block_at in [BlockAt::Idle, BlockAt::Setup, BlockAt::Response, BlockAt::Notification] {
                tracing::info!(?block_at, "cancel a heartbeat worker while its current wait is blocked");
                let (tx_notification, mut rx_notification) = C::mpsc(1);
                tx_notification.send(Notification::Tick { i: 1 }).await.unwrap();
                let fixture = Fixture::new(block_at, tx_notification);
                if block_at != BlockAt::Idle {
                    let heartbeat = HeartbeatEvent {
                        time: SendStamp::new(C::now(), 1),
                        matching: None,
                        cluster_committed: None,
                    };
                    fixture.event_tx.send(Some(heartbeat)).unwrap();
                }

                let (cancel, future) = fixture.worker.run();
                let mut future = std::pin::pin!(future);
                let pending = future.as_mut().now_or_never();
                assert_eq!(None, pending, "worker must block at {block_at:?}");
                let reached = fixture.reached.load(Ordering::Relaxed);
                let expected_reached = block_at != BlockAt::Idle;
                assert_eq!(expected_reached, reached, "worker must reach {block_at:?}");

                drop(cancel);
                C::timeout(TEST_TIMEOUT, future).await.unwrap();
                let dropped = fixture.dropped.load(Ordering::Relaxed);
                assert!(dropped, "cancellation must release the worker's network");

                let first = rx_notification.recv().await;
                let sentinel = matches!(first, Some(Notification::Tick { i: 1 }));
                assert!(sentinel);
                let next = rx_notification.recv().await;
                let closed = next.is_none();
                assert!(closed, "the cancelled worker must release its notification sender");
            }
        });
    }

    #[test]
    fn closing_the_event_channel_stops_the_worker() {
        C::run(async {
            let (tx_notification, _rx_notification) = C::mpsc(1);
            let fixture = Fixture::new(BlockAt::Idle, tx_notification);
            let (_cancel, future) = fixture.worker.run();
            drop(fixture.event_tx);
            C::timeout(TEST_TIMEOUT, future).await.unwrap();
            let dropped = fixture.dropped.load(Ordering::Relaxed);
            assert!(dropped, "event-channel closure must release the worker's network");
        });
    }
}
