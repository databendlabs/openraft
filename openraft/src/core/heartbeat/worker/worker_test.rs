use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::task::Poll;
use std::time::Duration;

use anyerror::AnyError;
use futures_util::FutureExt;
use futures_util::Stream;
use futures_util::StreamExt;
use tracing::Event;
use tracing::Level;
use tracing::Metadata;
use tracing::Subscriber;
use tracing::span;
use tracing::subscriber::NoSubscriber;

use super::HeartbeatWorker;
use crate::Config;
use crate::OptionalSend;
use crate::RaftTypeConfig;
use crate::async_runtime::MpscReceiver;
use crate::async_runtime::MpscSender;
use crate::async_runtime::OneshotSender;
use crate::async_runtime::watch::WatchSender;
use crate::base::BoxFuture;
use crate::base::BoxStream;
use crate::core::heartbeat::event::HeartbeatEvent;
use crate::core::notification::Notification;
use crate::engine::testing::UTConfig;
use crate::engine::testing::UTLeaderId;
use crate::engine::testing::log_id;
use crate::error::RPCError;
use crate::errors::NetworkError;
use crate::errors::Unreachable;
use crate::impls::Vote;
use crate::network::NetStreamAppend;
use crate::network::RPCOption;
use crate::progress::stream_id::StreamId;
use crate::proposer::SendStamp;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendError;
use crate::raft::AppendResult;
use crate::replication::response::ReplicationResult;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::CommittedVoteOf;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::MpscReceiverOf;
use crate::type_config::alias::MpscSenderOf;
use crate::type_config::alias::OneshotReceiverOf;
use crate::type_config::alias::VoteOf;
use crate::type_config::alias::WatchSenderOf;
use crate::type_config::async_runtime::hook_runtime::HookRuntime;
use crate::vote::raft_vote::RaftVoteExt;

type C = UTConfig;

const TEST_TIMEOUT: Duration = Duration::from_secs(1);
const BLOCKED_HEARTBEAT_MS: u64 = 3_600_000;
/// The `election_timeout_max` of a [`Fixture`]: how long its worker polls a heartbeat in flight
/// after its event channel closes.
const STOP_LIMIT_MS: u64 = 50;

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
            election_timeout_max: STOP_LIMIT_MS,
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

/// Dropping the cancel handle stops a worker at once while it waits for an event or for room in
/// the channel to RaftCore. While it sends a heartbeat that never ends, the worker stops once
/// `election_timeout_max` has passed. A stopped worker releases its network and its notification
/// sender.
#[test]
fn dropping_the_handle_ends_every_worker_wait() {
    C::run(async {
        for block_at in [BlockAt::Idle, BlockAt::Setup, BlockAt::Response, BlockAt::Notification] {
            tracing::info!(?block_at, "stop a heartbeat worker while its current wait is blocked");
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

            let stopped_at = C::now();
            drop(cancel);
            C::timeout(TEST_TIMEOUT, future).await.unwrap();
            if matches!(block_at, BlockAt::Setup | BlockAt::Response) {
                let waited = C::now() - stopped_at;
                let limit = Duration::from_millis(STOP_LIMIT_MS);
                assert!(
                    waited >= limit,
                    "the heartbeat in flight must get {limit:?} to end, got {waited:?}"
                );
            }
            let dropped = fixture.dropped.load(Ordering::Relaxed);
            assert!(dropped, "a stopped worker must release its network");

            let first = rx_notification.recv().await;
            let sentinel = matches!(first, Some(Notification::Tick { i: 1 }));
            assert!(sentinel);
            let next = rx_notification.recv().await;
            let closed = next.is_none();
            assert!(closed, "a stopped worker must release its notification sender");
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

/// How a scripted response stream behaves after its responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tail {
    /// The stream ends right after its responses.
    End,
    /// The stream never ends.
    Pending,
}

/// What a [`ScriptedNetwork`] does for one heartbeat.
struct Script {
    /// The error that fails the stream setup, instead of returning a response stream.
    setup_error: Option<RPCError<C>>,
    responses: Vec<Result<AppendResult<C>, RPCError<C>>>,
    tail: Tail,
}

impl Script {
    fn respond(responses: Vec<AppendResult<C>>, tail: Tail) -> Self {
        Self {
            setup_error: None,
            responses: responses.into_iter().map(Ok).collect(),
            tail,
        }
    }
}

/// A network that answers each heartbeat with the next [`Script`].
struct ScriptedNetwork {
    scripts: VecDeque<Script>,
}

impl NetStreamAppend<C> for ScriptedNetwork {
    fn stream_append<'s, S>(
        &'s mut self,
        _input: S,
        _option: RPCOption,
    ) -> BoxFuture<'s, Result<BoxStream<'s, Result<AppendResult<C>, RPCError<C>>>, RPCError<C>>>
    where
        S: Stream<Item = AppendEntriesRequest<C>> + OptionalSend + Unpin + 'static,
    {
        let script = self.scripts.pop_front().expect("a script for every heartbeat");
        if let Some(error) = script.setup_error {
            return Box::pin(async move { Err(error) });
        }

        let responses = futures_util::stream::iter(script.responses);
        let output: BoxStream<'s, _> = match script.tail {
            Tail::End => Box::pin(responses),
            Tail::Pending => Box::pin(responses.chain(futures_util::stream::pending())),
        };
        Box::pin(async move { Ok(output) })
    }
}

const SCRIPTED_HEARTBEAT_MS: u64 = 100;

fn leader_vote() -> CommittedVoteOf<C> {
    Vote::<UTLeaderId>::new(1, 1).to_committed()
}

fn higher_vote() -> VoteOf<C> {
    Vote::new_committed(9, 2)
}

fn unreachable() -> RPCError<C> {
    RPCError::Unreachable(Unreachable::new(&AnyError::error("unreachable")))
}

fn network_error() -> RPCError<C> {
    RPCError::Network(NetworkError::new(&AnyError::error("network error")))
}

/// Sends one heartbeat through a worker on `scripts`, then closes its event channel, so the
/// worker stops once it has handled the heartbeat. Returns the heartbeat and every notification
/// the worker sent, in order.
async fn run_heartbeat(scripts: Vec<Script>) -> (HeartbeatEvent<C>, Vec<Notification<C>>) {
    let (tx_notification, rx_notification) = C::mpsc(16);
    let (event_tx, rx) = C::watch_channel(None);
    let config = Config {
        heartbeat_interval: SCRIPTED_HEARTBEAT_MS,
        ..Config::default()
    };
    let worker = HeartbeatWorker {
        id: 1,
        leader_vote: leader_vote(),
        stream_id: StreamId::new(1),
        rx,
        network: ScriptedNetwork {
            scripts: scripts.into(),
        },
        target: 2,
        node: (),
        config: Arc::new(config),
        tx_notification,
    };

    let heartbeat = HeartbeatEvent {
        time: SendStamp::new(C::now(), 1),
        matching: Some(log_id(1, 1, 3)),
        cluster_committed: Some(log_id(1, 1, 2)),
    };
    event_tx.send(Some(heartbeat.clone())).unwrap();
    // The worker still consumes the event that was sent before the channel closed.
    drop(event_tx);

    let (_cancel, future) = worker.run();
    C::timeout(TEST_TIMEOUT, future).await.unwrap();
    let notifications = drain(rx_notification).await;
    (heartbeat, notifications)
}

/// Receives every notification until the channel closes.
async fn drain(mut rx_notification: MpscReceiverOf<C, Notification<C>>) -> Vec<Notification<C>> {
    let mut notifications = Vec::new();
    while let Some(notification) = rx_notification.recv().await {
        notifications.push(notification);
    }
    notifications
}

/// Checks that `notification` is the `HigherVote` of the scripted follower.
fn assert_higher_vote(notification: &Notification<C>) {
    let Notification::HigherVote {
        target,
        higher,
        leader_vote: originating,
    } = notification
    else {
        panic!("expected HigherVote, got: {notification}");
    };
    assert_eq!(2, *target);
    assert_eq!(higher_vote(), *higher);
    assert_eq!(leader_vote(), *originating);
}

/// Checks that `notification` is the `HeartbeatProgress` of `heartbeat`.
fn assert_progress(notification: &Notification<C>, heartbeat: &HeartbeatEvent<C>) {
    let Notification::HeartbeatProgress {
        stream_id,
        sending_time,
        target,
    } = notification
    else {
        panic!("expected HeartbeatProgress, got: {notification}");
    };
    assert_eq!(StreamId::new(1), *stream_id);
    assert_eq!(
        heartbeat.time, *sending_time,
        "the progress keeps the stamp of the heartbeat"
    );
    assert_eq!(2, *target);
}

/// A received higher vote reaches RaftCore even when the response stream does not end before the
/// heartbeat deadline.
#[test]
fn test_higher_vote_survives_a_stream_that_does_not_end() {
    C::run(async {
        let script = Script::respond(vec![Err(AppendError::HigherVote(higher_vote()))], Tail::Pending);
        let (_heartbeat, notifications) = run_heartbeat(vec![script]).await;

        assert_eq!(1, notifications.len(), "only the higher vote is reported");
        assert_higher_vote(&notifications[0]);
    });
}

/// A higher vote after one or more successes is reported, alone: it ends the leadership, so the
/// successes before it report no heartbeat progress.
#[test]
fn test_higher_vote_after_successes_is_reported_alone() {
    C::run(async {
        for successes in [1, 2] {
            tracing::info!(successes, "--- a higher vote after {successes} successful replies");
            {
                let mut responses = vec![Ok(None); successes];
                responses.push(Err(AppendError::HigherVote(higher_vote())));
                let script = Script::respond(responses, Tail::End);
                let (_heartbeat, notifications) = run_heartbeat(vec![script]).await;

                assert_eq!(1, notifications.len(), "only the higher vote is reported");
                assert_higher_vote(&notifications[0]);
            }
        }
    });
}

/// A success or a conflict is reported when the stream does not end before the deadline, and
/// successful replies report one heartbeat progress, with the stamp of the heartbeat.
#[test]
fn test_success_and_conflict_are_reported_once() {
    C::run(async {
        tracing::info!("--- three successes on a stream that does not end report one progress");
        {
            let script = Script::respond(vec![Ok(None), Ok(None), Ok(None)], Tail::Pending);
            let (heartbeat, notifications) = run_heartbeat(vec![script]).await;

            assert_eq!(1, notifications.len());
            assert_progress(&notifications[0], &heartbeat);
        }

        tracing::info!("--- a conflict on a stream that does not end reports the matching log id");
        {
            let conflict = AppendError::Conflict {
                log_id: log_id(1, 1, 3),
                hint: None,
            };
            let script = Script::respond(vec![Err(conflict)], Tail::Pending);
            let (heartbeat, notifications) = run_heartbeat(vec![script]).await;

            assert_eq!(2, notifications.len());
            let Notification::ReplicationProgress {
                stream_id,
                progress,
                inflight_id,
            } = &notifications[0]
            else {
                panic!("expected ReplicationProgress, got: {}", notifications[0]);
            };
            assert_eq!(StreamId::new(1), *stream_id);
            assert_eq!(None, *inflight_id);
            assert_eq!(2, progress.target);
            let Ok(ReplicationResult(Err(conflict))) = &progress.result else {
                panic!("expected a conflict, got: {}", progress);
            };
            assert_eq!(heartbeat.matching.as_ref(), Some(conflict));
            assert!(progress.hint.is_none());
            assert_progress(&notifications[1], &heartbeat);
        }
    });
}

/// No response, and a setup or a response that fails or times out, report nothing, and the worker
/// goes on.
#[test]
fn test_heartbeat_without_a_response_reports_nothing() {
    C::run(async {
        let cases = [
            ("empty stream", Script::respond(vec![], Tail::End)),
            ("unreachable at setup", Script {
                setup_error: Some(unreachable()),
                responses: vec![],
                tail: Tail::End,
            }),
            ("unreachable response", Script {
                setup_error: None,
                responses: vec![Err(unreachable())],
                tail: Tail::Pending,
            }),
            ("network error at setup", Script {
                setup_error: Some(network_error()),
                responses: vec![],
                tail: Tail::End,
            }),
            ("network error response", Script {
                setup_error: None,
                responses: vec![Err(network_error())],
                tail: Tail::Pending,
            }),
            (
                "no response before the deadline",
                Script::respond(vec![], Tail::Pending),
            ),
        ];
        for (case, script) in cases {
            tracing::info!(case, "--- a heartbeat without a response");
            {
                let (_heartbeat, notifications) = run_heartbeat(vec![script]).await;
                assert_eq!(0, notifications.len(), "{case} must report nothing");
            }
        }
    });
}

/// A higher vote waits for room in a full notification channel instead of being dropped, also
/// after the heartbeat deadline passed.
#[test]
fn test_higher_vote_waits_for_a_full_notification_channel() {
    C::run(async {
        let (tx_notification, mut rx_notification) = C::mpsc(1);
        tx_notification.send(Notification::Tick { i: 1 }).await.unwrap();
        let (event_tx, rx) = C::watch_channel(None);
        let config = Config {
            heartbeat_interval: SCRIPTED_HEARTBEAT_MS,
            ..Config::default()
        };
        let script = Script::respond(vec![Err(AppendError::HigherVote(higher_vote()))], Tail::End);
        let worker = HeartbeatWorker {
            id: 1,
            leader_vote: leader_vote(),
            stream_id: StreamId::new(1),
            rx,
            network: ScriptedNetwork {
                scripts: VecDeque::from([script]),
            },
            target: 2,
            node: (),
            config: Arc::new(config),
            tx_notification,
        };
        let heartbeat = HeartbeatEvent {
            time: SendStamp::new(C::now(), 1),
            matching: None,
            cluster_committed: None,
        };
        event_tx.send(Some(heartbeat)).unwrap();
        drop(event_tx);
        let (_cancel, future) = worker.run();
        let join = C::spawn(future);

        // Leave the channel full past the heartbeat deadline. The wait only makes the case
        // stronger; the order of the notifications does not depend on it.
        C::sleep(Duration::from_millis(2 * SCRIPTED_HEARTBEAT_MS)).await;

        let first = rx_notification.recv().await;
        let sentinel = matches!(first, Some(Notification::Tick { i: 1 }));
        assert!(sentinel);
        let second = C::timeout(TEST_TIMEOUT, rx_notification.recv()).await.unwrap().unwrap();
        assert_higher_vote(&second);
        C::timeout(TEST_TIMEOUT, join).await.unwrap().unwrap();
    });
}

/// A reply that an error follows is reported: the target replied.
#[test]
fn test_reply_before_an_error_is_reported() {
    C::run(async {
        let script = Script {
            setup_error: None,
            responses: vec![Ok(Ok(None)), Err(unreachable())],
            tail: Tail::End,
        };
        let (heartbeat, notifications) = run_heartbeat(vec![script]).await;

        assert_eq!(1, notifications.len());
        assert_progress(&notifications[0], &heartbeat);
    });
}

/// A tracing subscriber that records the level and the message of every event.
struct EventRecorder {
    events: Arc<Mutex<Vec<(Level, String)>>>,
}

impl Subscriber for EventRecorder {
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _span: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }

    fn record(&self, _span: &span::Id, _values: &span::Record<'_>) {}

    fn record_follows_from(&self, _span: &span::Id, _follows: &span::Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut message = MessageField(String::new());
        event.record(&mut message);
        let level = *event.metadata().level();
        self.events.lock().unwrap().push((level, message.0));
    }

    fn enter(&self, _span: &span::Id) {}

    fn exit(&self, _span: &span::Id) {}
}

/// The `message` field of an event.
struct MessageField(String);

impl tracing::field::Visit for MessageField {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

/// Only the first of the heartbeats that fail in a row logs a warning, whatever the failure. The
/// later ones log at debug level. The next reply logs at info level how many failed, or at debug
/// level after a single failure.
#[test]
fn test_only_the_first_failure_in_a_row_warns() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let recorder = EventRecorder { events: events.clone() };

    // tracing caches whether each log statement is enabled when a thread first runs it. With one
    // subscriber installed, it decides by the subscriber of that thread, so a statement that a test
    // thread without one runs first would be cached as disabled on this thread too. With a second
    // one installed before the recorder, every event asks the subscriber of its own thread.
    let _second = tracing::Dispatch::new(NoSubscriber::default());

    // `run()` polls the future on this thread, and the future polls the worker inline, so the
    // worker logs to the recorder.
    tracing::subscriber::with_default(recorder, || {
        C::run(async {
            let scripts = VecDeque::from([
                Script {
                    setup_error: Some(unreachable()),
                    responses: vec![],
                    tail: Tail::End,
                },
                Script {
                    setup_error: None,
                    responses: vec![Err(network_error())],
                    tail: Tail::End,
                },
                Script::respond(vec![Ok(None)], Tail::End),
                Script::respond(vec![], Tail::End),
                Script::respond(vec![Ok(None)], Tail::End),
                Script::respond(vec![], Tail::Pending),
            ]);
            let (tx_notification, _rx_notification) = C::mpsc(16);
            let (event_tx, rx) = C::watch_channel(None);
            let config = Config {
                heartbeat_interval: SCRIPTED_HEARTBEAT_MS,
                ..Config::default()
            };
            let worker = HeartbeatWorker {
                id: 1,
                leader_vote: leader_vote(),
                stream_id: StreamId::new(1),
                rx,
                network: ScriptedNetwork { scripts },
                target: 2,
                node: (),
                config: Arc::new(config),
                tx_notification,
            };
            let (_cancel, future) = worker.run();
            let mut future = std::pin::pin!(future);
            let event = |round| HeartbeatEvent {
                time: SendStamp::new(C::now(), round),
                matching: None,
                cluster_committed: None,
            };

            tracing::info!("--- two heartbeats fail, each for another reason, then a third gets a reply");
            {
                for round in 1..=3 {
                    event_tx.send(Some(event(round))).unwrap();
                    let pending = future.as_mut().now_or_never();
                    assert_eq!(None, pending, "the worker waits for the next heartbeat");
                }
            }

            tracing::info!("--- one heartbeat fails, then the next gets a reply");
            {
                for round in 4..=5 {
                    event_tx.send(Some(event(round))).unwrap();
                    let pending = future.as_mut().now_or_never();
                    assert_eq!(None, pending, "the worker waits for the next heartbeat");
                }
            }

            tracing::info!("--- the next heartbeat gets no response before its deadline");
            {
                event_tx.send(Some(event(6))).unwrap();
                drop(event_tx);
                C::timeout(TEST_TIMEOUT, future).await.unwrap();
            }
        })
    });

    let events = events.lock().unwrap();
    let logged: Vec<_> = events
        .iter()
        .filter(|(_, message)| message.contains("heartbeat failed") || message.contains("target replied"))
        .collect();
    let levels: Vec<_> = logged.iter().map(|(level, _)| *level).collect();
    assert_eq!(
        vec![
            Level::WARN,
            Level::DEBUG,
            Level::INFO,
            Level::WARN,
            Level::DEBUG,
            Level::WARN
        ],
        levels
    );
    let after_two = &logged[2].1;
    assert!(after_two.contains("after 2 failed heartbeats"), "got: {after_two}");
    let after_one = &logged[4].1;
    assert!(after_one.contains("after 1 failed heartbeat"), "got: {after_one}");
}

/// Sets its flag when it is dropped while it is armed.
struct DropAlarm {
    flag: Arc<AtomicBool>,
    armed: bool,
}

impl DropAlarm {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for DropAlarm {
    fn drop(&mut self) {
        if self.armed {
            self.flag.store(true, Ordering::Relaxed);
        }
    }
}

/// A network whose target answers each heartbeat with one success, once the test opens the gate
/// of the heartbeat.
struct GatedNetwork {
    /// The gate of each heartbeat, in order.
    gates: VecDeque<OneshotReceiverOf<C, ()>>,

    /// The index of the `prev_log_id` of each heartbeat sent, in order.
    sent: Arc<Mutex<Vec<u64>>>,

    /// Set when a response stream is dropped before it yields its reply.
    cut_short: Arc<AtomicBool>,
}

impl NetStreamAppend<C> for GatedNetwork {
    fn stream_append<'s, S>(
        &'s mut self,
        mut input: S,
        _option: RPCOption,
    ) -> BoxFuture<'s, Result<BoxStream<'s, Result<AppendResult<C>, RPCError<C>>>, RPCError<C>>>
    where
        S: Stream<Item = AppendEntriesRequest<C>> + OptionalSend + Unpin + 'static,
    {
        let request = input.next().now_or_never().flatten().expect("the heartbeat is ready");
        let matching = request.prev_log_id.expect("every heartbeat has a matching log id");
        self.sent.lock().unwrap().push(matching.index);

        let gate = self.gates.pop_front().expect("a gate for every heartbeat");
        let mut alarm = DropAlarm {
            flag: self.cut_short.clone(),
            armed: true,
        };
        let reply = async move {
            gate.await.expect("the test opens every gate");
            alarm.disarm();
            Ok(Ok(None))
        };
        let output: BoxStream<'s, _> = Box::pin(futures_util::stream::once(reply));
        Box::pin(async move { Ok(output) })
    }
}

/// A heartbeat without a reply in time stays in flight until its reply arrives: the worker
/// neither drops its response stream nor sends another heartbeat meanwhile, and it reports the
/// late reply. The heartbeat events that arrive meanwhile count as failed heartbeats, and the
/// latest of them goes out once the late heartbeat has ended.
#[test]
fn test_a_heartbeat_without_a_reply_in_time_stays_in_flight() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let recorder = EventRecorder { events: events.clone() };
    // See `test_only_the_first_failure_in_a_row_warns`.
    let _second = tracing::Dispatch::new(NoSubscriber::default());

    tracing::subscriber::with_default(recorder, || {
        C::run(async {
            let (open_first, first_gate) = C::oneshot();
            let (open_third, third_gate) = C::oneshot();
            let sent = Arc::new(Mutex::new(Vec::new()));
            let cut_short = Arc::new(AtomicBool::new(false));
            let network = GatedNetwork {
                gates: VecDeque::from([first_gate, third_gate]),
                sent: sent.clone(),
                cut_short: cut_short.clone(),
            };
            let (tx_notification, mut rx_notification) = C::mpsc(16);
            let (event_tx, rx) = C::watch_channel(None);
            let config = Config {
                heartbeat_interval: SCRIPTED_HEARTBEAT_MS,
                ..Config::default()
            };
            let worker = HeartbeatWorker {
                id: 1,
                leader_vote: leader_vote(),
                stream_id: StreamId::new(1),
                rx,
                network,
                target: 2,
                node: (),
                config: Arc::new(config),
                tx_notification,
            };
            let (_cancel, worker) = worker.run();

            let interval = Duration::from_millis(SCRIPTED_HEARTBEAT_MS);
            let event = |index| HeartbeatEvent {
                time: SendStamp::new(C::now(), index),
                matching: Some(log_id(1, 1, index)),
                cluster_committed: None,
            };

            // The test drives the worker on this thread, so that the recorder sees its logs.
            let test = async {
                let first = event(1);
                let third = event(3);

                tracing::info!("--- heartbeat 1 gets no reply in time, and events 2 and 3 arrive meanwhile");
                {
                    event_tx.send(Some(first.clone())).unwrap();
                    C::sleep(interval * 2).await;
                    event_tx.send(Some(event(2))).unwrap();
                    C::sleep(interval / 2).await;
                    event_tx.send(Some(third.clone())).unwrap();
                    C::sleep(interval / 2).await;

                    assert_eq!(
                        vec![1],
                        *sent.lock().unwrap(),
                        "no heartbeat goes out while one is in flight"
                    );
                    assert!(!cut_short.load(Ordering::Relaxed), "heartbeat 1 stays in flight");
                    assert!(
                        rx_notification.try_recv().is_err(),
                        "nothing is reported before a reply"
                    );
                }

                tracing::info!("--- the late reply to heartbeat 1 arrives, then heartbeat 3 goes out");
                {
                    open_first.send(()).unwrap();
                    let late = C::timeout(TEST_TIMEOUT, rx_notification.recv()).await.unwrap().unwrap();
                    assert_progress(&late, &first);

                    open_third.send(()).unwrap();
                    let in_time = C::timeout(TEST_TIMEOUT, rx_notification.recv()).await.unwrap().unwrap();
                    assert_progress(&in_time, &third);
                    assert_eq!(vec![1, 3], *sent.lock().unwrap(), "event 2 is replaced by event 3");
                }

                drop(event_tx);
            };

            let (stopped, ()) = futures_util::future::join(C::timeout(TEST_TIMEOUT * 5, worker), test).await;
            stopped.expect("closing the event channel stops the worker");
            assert!(
                !cut_short.load(Ordering::Relaxed),
                "no response stream is dropped before its reply"
            );
        })
    });

    // Heartbeat 1 fails at its deadline, each event that arrives while it is in flight counts as
    // another failure, and the reply in time to heartbeat 3 ends them. Events 2 and 3 count as one
    // failure or two, as the worker sees them apart or together.
    let events = events.lock().unwrap();
    let logged: Vec<_> = events.iter().filter(|(_, message)| message.starts_with("id=1 target=2 ")).collect();
    let levels: Vec<_> = logged.iter().map(|(level, _)| *level).collect();
    assert_eq!(Some(&Level::WARN), levels.first(), "got: {logged:?}");
    assert_eq!(Some(&Level::INFO), levels.last(), "got: {logged:?}");
    assert!(
        levels[1..levels.len() - 1].iter().all(|level| *level == Level::DEBUG),
        "got: {logged:?}"
    );
    let late = logged.iter().filter(|(_, message)| message.contains("late reply")).count();
    assert_eq!(1, late, "got: {logged:?}");
    let replied = &logged[logged.len() - 1].1;
    assert!(replied.contains("failed heartbeats"), "got: {replied}");
}

/// A worker stopped while a heartbeat is in flight lets the heartbeat end instead of dropping it,
/// then ends without reporting the reply.
#[test]
fn test_a_stopped_worker_lets_its_heartbeat_end() {
    C::run(async {
        let (open, gate) = C::oneshot();
        let sent = Arc::new(Mutex::new(Vec::new()));
        let cut_short = Arc::new(AtomicBool::new(false));
        let network = GatedNetwork {
            gates: VecDeque::from([gate]),
            sent: sent.clone(),
            cut_short: cut_short.clone(),
        };
        let (tx_notification, mut rx_notification) = C::mpsc(16);
        let (event_tx, rx) = C::watch_channel(None);
        // Neither the heartbeat's deadline nor the limit after the stop passes in this test.
        let config = Config {
            heartbeat_interval: BLOCKED_HEARTBEAT_MS,
            election_timeout_max: BLOCKED_HEARTBEAT_MS,
            ..Config::default()
        };
        let worker = HeartbeatWorker {
            id: 1,
            leader_vote: leader_vote(),
            stream_id: StreamId::new(1),
            rx,
            network,
            target: 2,
            node: (),
            config: Arc::new(config),
            tx_notification,
        };
        let (cancel, worker) = worker.run();
        let mut worker = std::pin::pin!(worker);

        tracing::info!("--- heartbeat 1 is in flight when the worker stops");
        {
            let heartbeat = HeartbeatEvent {
                time: SendStamp::new(C::now(), 1),
                matching: Some(log_id(1, 1, 1)),
                cluster_committed: None,
            };
            event_tx.send(Some(heartbeat)).unwrap();
            assert_eq!(None, worker.as_mut().now_or_never(), "heartbeat 1 waits for its gate");
            assert_eq!(vec![1], *sent.lock().unwrap());

            drop(cancel);
            assert_eq!(
                None,
                worker.as_mut().now_or_never(),
                "the stopped worker waits for heartbeat 1"
            );
        }

        tracing::info!("--- the reply to heartbeat 1 arrives, and the worker ends");
        {
            open.send(()).unwrap();
            C::timeout(TEST_TIMEOUT, worker).await.unwrap();
            assert!(
                !cut_short.load(Ordering::Relaxed),
                "heartbeat 1 is not dropped before its reply"
            );
            let reported = rx_notification.recv().await;
            assert!(reported.is_none(), "a stopped worker reports nothing");
        }
    });
}

/// A type config on [`HookRuntime`]: Tokio, with a watch receiver that can run a hook right after
/// `changed()`, to place a send between the worker's change notification and its read.
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd)]
struct HookConfig;

impl RaftTypeConfig for HookConfig {
    type D = u64;
    type R = ();
    type NodeId = u64;
    type Node = ();
    type Term = u64;
    type LeaderId = UTLeaderId;
    type Vote = Vote<Self::LeaderId>;
    type Membership = crate::impls::Membership<Self::NodeId, Self::Node>;
    type MembershipEdit = crate::impls::ChangeMembers<Self::NodeId, Self::Node>;
    type Payload = crate::impls::EntryPayload<Self::D, Self::Membership>;
    type Entry = crate::impls::Entry<<Self::LeaderId as crate::vote::RaftLeaderId>::Committed, Self::Payload>;
    type AsyncRuntime = HookRuntime;
    type Responder<T>
        = crate::impls::OneshotResponder<Self, T>
    where T: OptionalSend + 'static;
    type Batch<T>
        = crate::impls::InlineBatch<T>
    where T: OptionalSend + 'static;
    type ErrorSource = AnyError;
}

/// A network that records each heartbeat request and answers it with one success.
struct RecordingNetwork {
    sent: Arc<Mutex<Vec<AppendEntriesRequest<HookConfig>>>>,
}

impl NetStreamAppend<HookConfig> for RecordingNetwork {
    fn stream_append<'s, S>(
        &'s mut self,
        mut input: S,
        _option: RPCOption,
    ) -> BoxFuture<
        's,
        Result<BoxStream<'s, Result<AppendResult<HookConfig>, RPCError<HookConfig>>>, RPCError<HookConfig>>,
    >
    where
        S: Stream<Item = AppendEntriesRequest<HookConfig>> + OptionalSend + Unpin + 'static,
    {
        let sent = self.sent.clone();
        let output = futures_util::stream::poll_fn(move |cx| match input.poll_next_unpin(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(request)) => {
                sent.lock().unwrap().push(request);
                Poll::Ready(Some(Ok(Ok(None))))
            }
            Poll::Ready(None) => Poll::Ready(None),
        });
        let output: BoxStream<'s, _> = Box::pin(output);
        Box::pin(async move { Ok(output) })
    }
}

fn hook_event(round: u64, matching: u64) -> HeartbeatEvent<HookConfig> {
    HeartbeatEvent {
        time: SendStamp::new(HookConfig::now(), round),
        matching: Some(crate::testing::log_id::<HookConfig>(1, 1, matching)),
        cluster_committed: Some(crate::testing::log_id::<HookConfig>(1, 1, matching - 1)),
    }
}

/// The vote, the previous log id, the commit and the entry count of a heartbeat request.
type RequestFields = (
    VoteOf<HookConfig>,
    Option<LogIdOf<HookConfig>>,
    Option<LogIdOf<HookConfig>>,
    usize,
);

fn request_fields(request: &AppendEntriesRequest<HookConfig>) -> RequestFields {
    (
        request.vote,
        request.prev_log_id,
        request.leader_commit,
        request.entries.len(),
    )
}

/// The fields of the heartbeat request that leader 1 in term 1 sends for `event`.
fn expected_request(event: &HeartbeatEvent<HookConfig>) -> RequestFields {
    let vote = Vote::<UTLeaderId>::new_committed(1, 1);
    (vote, event.matching, event.cluster_committed, 0)
}

/// An event that arrives between the worker's change notification and its read is sent once:
/// the worker marks the event it reads as seen, so it then waits for a new event instead of sending
/// the same one again.
#[test]
fn test_event_sent_before_the_read_is_sent_once() {
    HookConfig::run(async {
        let (tx_notification, _rx_notification) = HookConfig::mpsc(16);
        let (event_tx, rx) = HookConfig::watch_channel(None);
        let sent = Arc::new(Mutex::new(Vec::new()));

        let event_b = hook_event(2, 4);
        let hook_tx = event_tx.clone();
        let hook_b = event_b.clone();
        rx.after_changed(move || {
            hook_tx.send(Some(hook_b)).unwrap();
        });

        let config = Config {
            heartbeat_interval: SCRIPTED_HEARTBEAT_MS,
            ..Config::default()
        };
        let worker = HeartbeatWorker {
            id: 1,
            leader_vote: Vote::<UTLeaderId>::new(1, 1).to_committed(),
            stream_id: StreamId::new(1),
            rx,
            network: RecordingNetwork { sent: sent.clone() },
            target: 2,
            node: (),
            config: Arc::new(config),
            tx_notification,
        };
        let (_cancel, future) = worker.run();
        let mut future = std::pin::pin!(future);

        tracing::info!("--- A wakes the worker, and B is sent before the worker reads the event");
        {
            event_tx.send(Some(hook_event(1, 3))).unwrap();
            let pending = future.as_mut().now_or_never();
            assert_eq!(None, pending, "the worker waits for the next event");
            let requests: Vec<_> = sent.lock().unwrap().drain(..).map(|r| request_fields(&r)).collect();
            assert_eq!(
                vec![expected_request(&event_b)],
                requests,
                "B is sent once, and A never"
            );
        }

        tracing::info!("--- C is the next event, and the worker sends it once");
        {
            let event_c = hook_event(3, 5);
            event_tx.send(Some(event_c.clone())).unwrap();
            let pending = future.as_mut().now_or_never();
            assert_eq!(None, pending, "the worker waits for the next event");
            let requests: Vec<_> = sent.lock().unwrap().drain(..).map(|r| request_fields(&r)).collect();
            assert_eq!(vec![expected_request(&event_c)], requests);
        }

        drop(event_tx);
        let finished = future.as_mut().now_or_never();
        assert_eq!(Some(()), finished, "closing the event channel stops the worker");
    });
}
