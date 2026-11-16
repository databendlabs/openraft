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
    ) -> BoxFuture<'s, Result<BoxStream<'s, Result<AppendResult<C>, RPCError<C>>>, RPCError<C>>>
    where
        S: Stream<Item = AppendEntriesRequest<C>> + OptionalSend + Unpin + 'static,
    {
        let drained = self.drained.clone();
        let output = futures_util::stream::poll_fn(move |cx| match input.poll_next_unpin(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(_request)) => Poll::Ready(Some(Ok(Ok(None)))),
            Poll::Ready(None) => {
                drained.store(true, Ordering::Relaxed);
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

        tracing::info!(target = 2, "--- spawn a heartbeat worker");
        let (tx_event, _cancel) = {
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
            let (cancel, future) = worker.into_cancellable();
            C::spawn(future);

            (tx_event, cancel)
        };

        tracing::info!("--- send one heartbeat and wait for the worker to report its progress");
        {
            let heartbeat = HeartbeatEvent {
                time: SendStamp::new(C::now(), 1),
                matching: None,
                cluster_committed: None,
            };
            tx_event.send(Some(heartbeat)).unwrap();

            let notification = rx_notification.recv().await.unwrap();
            assert!(matches!(notification, Notification::HeartbeatProgress { .. }));
        }

        tracing::info!("--- the response stream must be drained before the progress is reported");
        let drained = drained.load(Ordering::Relaxed);
        assert!(drained);
    });
}

const TEST_TIMEOUT: Duration = Duration::from_secs(1);
const BLOCKED_HEARTBEAT_MS: u64 = 3_600_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockAt {
    Idle,
    Setup,
    Response,
    Notification,
}

struct BlockingNetwork {
    block_at: BlockAt,
    reached: Arc<AtomicBool>,
    dropped: Arc<AtomicBool>,
}

impl Drop for BlockingNetwork {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::Relaxed);
    }
}

impl NetStreamAppend<C> for BlockingNetwork {
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
    worker: HeartbeatWorker<C, BlockingNetwork>,
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
        let network = BlockingNetwork {
            block_at,
            reached: reached.clone(),
            dropped: dropped.clone(),
        };
        let config = Config {
            heartbeat_interval: BLOCKED_HEARTBEAT_MS,
            ..Config::default()
        };
        let worker = HeartbeatWorker {
            id: 1,
            leader_vote,
            stream_id: StreamId::new(1),
            rx,
            network,
            target: 2,
            node: (),
            config: Arc::new(config),
            tx_notification,
        };
        Self {
            worker,
            event_tx,
            reached,
            dropped,
        }
    }
}

#[test]
fn cancellation_interrupts_every_worker_wait() {
    C::run(async {
        for block_at in [BlockAt::Idle, BlockAt::Setup, BlockAt::Response, BlockAt::Notification] {
            tracing::info!(?block_at, "cancel a heartbeat worker while its current wait is blocked");
            let (tx_notification, mut rx_notification) = C::mpsc(1);
            if block_at == BlockAt::Notification {
                tx_notification.send(Notification::Tick { i: 1 }).await.unwrap();
            }
            let fixture = Fixture::new(block_at, tx_notification);
            if block_at != BlockAt::Idle {
                let heartbeat = HeartbeatEvent {
                    time: SendStamp::new(C::now(), 1),
                    matching: None,
                    cluster_committed: None,
                };
                fixture.event_tx.send(Some(heartbeat)).unwrap();
            }

            let (cancel, future) = fixture.worker.into_cancellable();
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

            if block_at == BlockAt::Notification {
                let first = rx_notification.recv().await;
                let sentinel = matches!(first, Some(Notification::Tick { i: 1 }));
                assert!(sentinel);
                let next = rx_notification.recv().await;
                let closed = next.is_none();
                assert!(closed, "the blocked heartbeat notification must be dropped");
            }
        }
    });
}

#[test]
fn closing_the_event_channel_stops_the_worker() {
    C::run(async {
        let (tx_notification, _rx_notification) = C::mpsc(1);
        let fixture = Fixture::new(BlockAt::Idle, tx_notification);
        let (_cancel, future) = fixture.worker.into_cancellable();
        drop(fixture.event_tx);
        C::timeout(TEST_TIMEOUT, future).await.unwrap();
        let dropped = fixture.dropped.load(Ordering::Relaxed);
        assert!(dropped, "event-channel closure must release the worker's network");
    });
}
