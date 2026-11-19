use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::io;
use std::ops::Bound;
use std::ops::RangeBounds;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use futures_util::Stream;
use futures_util::TryStreamExt;

use crate::OptionalSend;
use crate::RaftLogReader;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::async_runtime::MpscReceiver;
use crate::async_runtime::MpscSender;
use crate::async_runtime::OneshotSender;
use crate::async_runtime::TryRecvError;
use crate::async_runtime::watch::WatchReceiver;
use crate::async_runtime::watch::WatchSender;
use crate::core::ApplyResult;
use crate::core::notification::Notification;
use crate::core::sm::Command;
use crate::core::sm::CommandResult;
use crate::core::sm::Response;
use crate::core::sm::apply_batch::ApplyBatch;
use crate::core::sm::command_receiver::CommandReceiver;
use crate::core::sm::worker::Worker;
use crate::core::sm::worker::intake::Intake;
use crate::core::sm::worker::progress::Progress;
use crate::core::sm::worker::work::Work;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::entry::RaftEntry;
use crate::impls::OneshotResponder;
use crate::raft::ClientWriteResult;
use crate::raft::responder::core_responder::CoreResponder;
use crate::storage::EntryResponder;
use crate::storage::RaftStateMachine;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::EntryOf;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::MpscReceiverOf;
use crate::type_config::alias::MpscSenderOf;
use crate::type_config::alias::OneshotReceiverOf;
use crate::type_config::alias::OneshotSenderOf;
use crate::type_config::alias::SnapshotMetaOf;
use crate::type_config::alias::SnapshotOf;
use crate::type_config::alias::StoredMembershipOf;

type Events = Arc<Mutex<Vec<String>>>;

const WAIT_TIMEOUT: Duration = Duration::from_secs(5);

/// Gates that hold an `apply()` or a read until the test releases them.
///
/// A gate is keyed by the stage and the first index of its range, such as `apply 2` or `read 2`.
#[derive(Clone, Default)]
struct Gates {
    gates: Arc<Mutex<BTreeMap<String, OneshotReceiverOf<UTConfig, ()>>>>,
    failures: Arc<Mutex<BTreeSet<String>>>,
}

impl Gates {
    /// Make the stage starting at `index` wait until the returned sender is dropped.
    fn gate(&self, stage: &str, index: u64) -> OneshotSenderOf<UTConfig, ()> {
        let (tx, rx) = UTConfig::<()>::oneshot();
        self.gates.lock().unwrap().insert(format!("{stage} {index}"), rx);
        tx
    }

    /// Wait for the gate of the stage starting at `index`, if the test set one.
    async fn pass(&self, stage: &str, index: u64) -> io::Result<()> {
        let gate = self.gates.lock().unwrap().remove(&format!("{stage} {index}"));
        if let Some(gate) = gate {
            let _released = gate.await;
        }
        let fail = self.failures.lock().unwrap().remove(&format!("{stage} {index}"));
        if fail {
            return Err(io::Error::other(format!("injected {stage} {index} failure")));
        }
        Ok(())
    }

    fn fail(&self, stage: &str, index: u64) {
        self.failures.lock().unwrap().insert(format!("{stage} {index}"));
    }
}

/// Records `apply [first,last]` once `apply()` has consumed its entries, and `done [first,last]`
/// when it returns; `ExternalFunc` commands record `func`.
///
/// An `apply()` with an `apply` gate on its first index waits for the gate before it returns.
#[derive(Clone, Default)]
struct RecordingStateMachine {
    events: Events,
    gates: Gates,
}

impl RaftStateMachine<UTConfig> for RecordingStateMachine {
    type SnapshotData = ();
    type SnapshotBuilder = ();

    async fn applied_state(&mut self) -> Result<(Option<LogIdOf<UTConfig>>, StoredMembershipOf<UTConfig>), io::Error> {
        Ok((None, StoredMembershipOf::<UTConfig>::default()))
    }

    async fn apply<Strm>(&mut self, mut entries: Strm) -> Result<(), io::Error>
    where Strm: Stream<Item = Result<EntryResponder<UTConfig>, io::Error>> + Unpin + OptionalSend {
        let mut indexes = Vec::new();
        while let Some((entry, responder)) = entries.try_next().await? {
            indexes.push(entry.index());
            if let Some(responder) = responder {
                responder.send(());
            }
        }
        let range = format!("[{},{}]", indexes[0], indexes[indexes.len() - 1]);
        self.events.lock().unwrap().push(format!("apply {range}"));

        self.gates.pass("apply", indexes[0]).await?;
        self.events.lock().unwrap().push(format!("done {range}"));
        Ok(())
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {}

    async fn install_snapshot(&mut self, _meta: &SnapshotMetaOf<UTConfig>, _snapshot: ()) -> Result<(), io::Error> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    async fn get_current_snapshot(&mut self) -> Result<Option<SnapshotOf<UTConfig, ()>>, io::Error> {
        let event = "snapshot".to_string();
        let mut events = self.events.lock().unwrap();
        events.push(event);
        Ok(None)
    }
}

/// Returns a blank entry of term 1 for every index in the requested range, and records each
/// read as `read [first,last]`.
///
/// A read with a `read` gate on its first index waits for the gate before it returns.
struct BlankLogReader {
    events: Events,
    gates: Gates,
}

impl RaftLogReader<UTConfig> for BlankLogReader {
    async fn try_get_log_entries<RB>(&mut self, range: RB) -> Result<Vec<EntryOf<UTConfig>>, io::Error>
    where RB: RangeBounds<u64> + Clone + std::fmt::Debug + OptionalSend {
        let start = match range.start_bound() {
            Bound::Included(start) => *start,
            Bound::Excluded(start) => start + 1,
            Bound::Unbounded => 0,
        };
        let end = match range.end_bound() {
            Bound::Included(end) => end + 1,
            Bound::Excluded(end) => *end,
            Bound::Unbounded => unreachable!("the worker reads bounded ranges"),
        };
        let event = format!("read [{},{}]", start, end - 1);
        self.events.lock().unwrap().push(event);
        self.gates.pass("read", start).await?;
        let entries = (start..end).map(|index| EntryOf::<UTConfig>::new_blank(log_id(1, 0, index)));
        Ok(entries.collect())
    }

    async fn read_vote(&mut self) -> Result<Option<<UTConfig as RaftTypeConfig>::Vote>, io::Error> {
        Ok(None)
    }
}

fn apply_cmd(
    first: u64,
    last: u64,
) -> (
    Command<UTConfig, RecordingStateMachine>,
    OneshotReceiverOf<UTConfig, ClientWriteResult<UTConfig>>,
) {
    let (responder, rx) = OneshotResponder::new_pair();
    let responders = vec![(last, CoreResponder::UserDefined(responder))];
    let cmd = Command::apply(log_id(1, 0, first), log_id(1, 0, last), responders);
    (cmd, rx)
}

fn record_func(name: &'static str) -> Command<UTConfig, RecordingStateMachine> {
    Command::ExternalFunc {
        func: Box::new(move |sm: &mut RecordingStateMachine| {
            let events = sm.events.clone();
            Box::pin(async move { events.lock().unwrap().push(name.to_string()) })
        }),
    }
}

/// Spawn a worker on `sm` with a log reader that records into `sm.events`.
fn spawn_worker(
    sm: RecordingStateMachine,
    cmd_tx: MpscSenderOf<UTConfig, Command<UTConfig, RecordingStateMachine>>,
    cmd_rx: MpscReceiverOf<UTConfig, Command<UTConfig, RecordingStateMachine>>,
    resp_tx: MpscSenderOf<UTConfig, Notification<UTConfig>>,
) -> impl Sized {
    let log_reader = BlankLogReader {
        events: sm.events.clone(),
        gates: sm.gates.clone(),
    };
    let cmd_rx = CommandReceiver::new(cmd_rx, false);
    let span = tracing::Span::none();
    Worker::spawn(0, sm, log_reader, resp_tx, (cmd_tx, cmd_rx), span)
}

/// Receive the next `Apply` response of the worker as `(since, end, last_applied)`.
async fn recv_applied(
    resp_rx: &mut MpscReceiverOf<UTConfig, Notification<UTConfig>>,
) -> anyhow::Result<(u64, u64, LogIdOf<UTConfig>)> {
    let notification = resp_rx.recv().await.expect("the worker sends one response per batch");
    let Notification::StateMachine { command_result } = notification else {
        panic!("unexpected notification: {}", notification);
    };
    let Response::Apply(ApplyResult {
        since,
        end,
        last_applied,
    }) = command_result.result?
    else {
        panic!("unexpected state machine response");
    };
    Ok((since, end, last_applied))
}

/// Wait until `events` contains `event`.
async fn wait_event(events: &Events, event: &str) {
    for _ in 0..5_000 {
        if events.lock().unwrap().iter().any(|e| e == event) {
            return;
        }
        UTConfig::<()>::sleep(Duration::from_millis(1)).await;
    }
    panic!("timed out waiting for {event:?}, got: {:?}", events.lock().unwrap());
}

/// Contiguous queued `Apply` commands run as one `apply()` call, and merging stops at the first
/// other command, which keeps its place in the order.
///
/// `UTConfig::run()` provides the `LocalSet` that spawning needs with the `single-threaded`
/// feature.
#[test]
fn merge_queued_apply_commands() -> anyhow::Result<()> {
    UTConfig::<()>::run(merge_queued_apply_commands_async())
}

async fn merge_queued_apply_commands_async() -> anyhow::Result<()> {
    let sm = RecordingStateMachine::default();
    let events = sm.events.clone();
    let (cmd_tx, cmd_rx) = UTConfig::<()>::mpsc(16);
    let (resp_tx, mut resp_rx) = UTConfig::<()>::mpsc(16);

    tracing::info!("--- queue Apply [1,2], [3,3], ExternalFunc, Apply [4,4] before the worker starts");
    let client_rxs = {
        let (cmd_1, rx_1) = apply_cmd(1, 2);
        let (cmd_2, rx_2) = apply_cmd(3, 3);
        let (cmd_3, rx_3) = apply_cmd(4, 4);
        for cmd in [cmd_1, cmd_2, record_func("func"), cmd_3] {
            let sent = cmd_tx.send(cmd).await;
            assert!(sent.is_ok(), "the channel has room for every command");
        }
        [rx_1, rx_2, rx_3]
    };

    tracing::info!("--- start the worker and collect its Apply responses");
    {
        let _handle = spawn_worker(sm, cmd_tx, cmd_rx, resp_tx);

        let mut applied = Vec::new();
        for _ in 0..2 {
            applied.push(recv_applied(&mut resp_rx).await?);
        }
        let want = vec![(1, 4, log_id(1, 0, 3)), (4, 5, log_id(1, 0, 4))];
        assert_eq!(want, applied);

        let got = events.lock().unwrap().clone();
        let want = vec![
            "read [1,3]",
            "apply [1,3]",
            "done [1,3]",
            "func",
            "read [4,4]",
            "apply [4,4]",
            "done [4,4]",
        ];
        assert_eq!(want, got);
    }

    tracing::info!("--- every client of a merged command receives its response");
    {
        let mut responded = Vec::new();
        for rx in client_rxs {
            let response = rx.await??;
            responded.push(response.log_id);
        }
        let want = vec![log_id(1, 0, 2), log_id(1, 0, 3), log_id(1, 0, 4)];
        assert_eq!(want, responded);
    }

    Ok(())
}

/// The reader fills the loaded channel while an earlier apply is held. The applier coalesces
/// that queued prefix without waiting for a later read, and preserves the function barrier.
#[test]
fn read_next_batch_while_applying() -> anyhow::Result<()> {
    UTConfig::<()>::run(read_next_batch_while_applying_async())
}

async fn read_next_batch_while_applying_async() -> anyhow::Result<()> {
    let sm = RecordingStateMachine::default();
    let events = sm.events.clone();
    let gates = sm.gates.clone();
    let (cmd_tx, cmd_rx) = UTConfig::<()>::mpsc(16);
    let (resp_tx, mut resp_rx) = UTConfig::<()>::mpsc(16);
    let apply_1 = gates.gate("apply", 1);
    let apply_2 = gates.gate("apply", 2);
    let read_5 = gates.gate("read", 5);
    let _handle = spawn_worker(sm, cmd_tx.clone(), cmd_rx, resp_tx);

    tracing::info!("--- hold A applying, then read and queue B and C in the loaded channel");
    {
        let (command, _client) = apply_cmd(1, 1);
        let sent = cmd_tx.send(command).await;
        let accepted = sent.is_ok();
        assert!(accepted);
        wait_event(&events, "apply [1,1]").await;
        let (command, _client) = apply_cmd(2, 3);
        let sent = cmd_tx.send(command).await;
        let accepted = sent.is_ok();
        assert!(accepted);
        wait_event(&events, "read [2,3]").await;
        let (command, _client) = apply_cmd(4, 4);
        let sent = cmd_tx.send(command).await;
        let accepted = sent.is_ok();
        assert!(accepted);
        wait_event(&events, "read [4,4]").await;
    }

    tracing::info!("--- reader D starts only after B and C were forwarded; hold D before the FIFO barrier");
    {
        let (command, _client) = apply_cmd(5, 5);
        let sent = cmd_tx.send(command).await;
        let accepted = sent.is_ok();
        assert!(accepted);
        wait_event(&events, "read [5,5]").await;
        let (tail, _client) = apply_cmd(6, 6);
        for command in [record_func("func"), tail] {
            let sent = cmd_tx.send(command).await;
            let accepted = sent.is_ok();
            assert!(accepted);
        }
        drop(apply_1);
        let first = recv_applied(&mut resp_rx).await?;
        let last = log_id(1, 0, 1);
        let expected = (1, 2, last);
        assert_eq!(expected, first);
        wait_event(&events, "apply [2,4]").await;
    }

    tracing::info!("--- apply the already-queued loaded prefix before releasing D");
    {
        drop(apply_2);
        let next = recv_applied(&mut resp_rx).await?;
        let last = log_id(1, 0, 4);
        let expected = (2, 5, last);
        assert_eq!(expected, next);
        let actual = event_snapshot(&events);
        let expected = vec![
            "read [1,1]",
            "apply [1,1]",
            "read [2,3]",
            "read [4,4]",
            "read [5,5]",
            "done [1,1]",
            "apply [2,4]",
            "done [2,4]",
        ];
        assert_eq!(expected, actual);
        drop(read_5);
    }

    tracing::info!("--- D applies before the function permits the final log read");
    {
        let next = recv_applied(&mut resp_rx).await?;
        let last = log_id(1, 0, 5);
        let expected = (5, 6, last);
        assert_eq!(expected, next);
        let tail = recv_applied(&mut resp_rx).await?;
        let last = log_id(1, 0, 6);
        let expected = (6, 7, last);
        assert_eq!(expected, tail);
        let actual = event_snapshot(&events);
        let expected = vec![
            "read [1,1]",
            "apply [1,1]",
            "read [2,3]",
            "read [4,4]",
            "read [5,5]",
            "done [1,1]",
            "apply [2,4]",
            "done [2,4]",
            "apply [5,5]",
            "done [5,5]",
            "func",
            "read [6,6]",
            "apply [6,6]",
            "done [6,6]",
        ];
        assert_eq!(expected, actual);
    }
    Ok(())
}

/// A completed apply is published while the following read remains gated, and later commands
/// retain their order after the read resumes.
#[test]
fn publish_apply_before_next_read_finishes() -> anyhow::Result<()> {
    UTConfig::<()>::run(publish_apply_before_next_read_finishes_async())
}

async fn publish_apply_before_next_read_finishes_async() -> anyhow::Result<()> {
    let sm = RecordingStateMachine::default();
    let events = sm.events.clone();
    let gates = sm.gates.clone();
    let (cmd_tx, cmd_rx) = UTConfig::<()>::mpsc(16);
    let (resp_tx, mut resp_rx) = UTConfig::<()>::mpsc(16);
    let read_2 = gates.gate("read", 2);
    let apply_1 = gates.gate("apply", 1);
    let _handle = spawn_worker(sm, cmd_tx.clone(), cmd_rx, resp_tx);

    tracing::info!("--- start read [1,1], then queue a separate read-ahead batch [2,2]");
    {
        let (cmd, _rx) = apply_cmd(1, 1);
        let sent = cmd_tx.send(cmd).await;
        assert!(sent.is_ok(), "the worker is running");
        wait_event(&events, "apply [1,1]").await;
        let (cmd, _rx) = apply_cmd(2, 2);
        let sent = cmd_tx.send(cmd).await;
        assert!(sent.is_ok(), "the worker is running");
        wait_event(&events, "read [2,2]").await;
    }

    tracing::info!("--- complete apply [1,1] while read [2,2] stays held");
    {
        let cmd = record_func("func");
        let sent = cmd_tx.send(cmd).await;
        assert!(sent.is_ok(), "the worker is running");
        let (cmd, _rx) = apply_cmd(3, 3);
        let sent = cmd_tx.send(cmd).await;
        assert!(sent.is_ok(), "the worker is running");
        drop(apply_1);
        let applied = UTConfig::<()>::timeout(WAIT_TIMEOUT, recv_applied(&mut resp_rx)).await??;
        assert_eq!((1, 2, log_id(1, 0, 1)), applied);
        let got = events.lock().unwrap().clone();
        let want = vec!["read [1,1]", "apply [1,1]", "read [2,2]", "done [1,1]"];
        assert_eq!(want, got);
    }

    tracing::info!("--- release read [2,2]; apply [2,2], the function and apply [3,3] retain FIFO order");
    {
        drop(read_2);
        let applied = UTConfig::<()>::timeout(WAIT_TIMEOUT, recv_applied(&mut resp_rx)).await??;
        assert_eq!((2, 3, log_id(1, 0, 2)), applied);
        let applied = UTConfig::<()>::timeout(WAIT_TIMEOUT, recv_applied(&mut resp_rx)).await??;
        assert_eq!((3, 4, log_id(1, 0, 3)), applied);
        let got = events.lock().unwrap().clone();
        let want = vec![
            "read [1,1]",
            "apply [1,1]",
            "read [2,2]",
            "done [1,1]",
            "apply [2,2]",
            "done [2,2]",
            "func",
            "read [3,3]",
            "apply [3,3]",
            "done [3,3]",
        ];
        assert_eq!(want, got);
    }
    Ok(())
}

/// An apply failure cancels a later read instead of waiting for that read to finish.
#[test]
fn apply_error_cancels_next_read() -> anyhow::Result<()> {
    UTConfig::<()>::run(apply_error_cancels_next_read_async())
}

async fn apply_error_cancels_next_read_async() -> anyhow::Result<()> {
    let sm = RecordingStateMachine::default();
    let events = sm.events.clone();
    let gates = sm.gates.clone();
    let (cmd_tx, cmd_rx) = UTConfig::<()>::mpsc(16);
    let (resp_tx, mut resp_rx) = UTConfig::<()>::mpsc(16);
    let read_2 = gates.gate("read", 2);
    let apply_1 = gates.gate("apply", 1);
    gates.fail("apply", 1);
    let _handle = spawn_worker(sm, cmd_tx.clone(), cmd_rx, resp_tx);

    tracing::info!("--- hold apply [1,1] and the later read [2,2]");
    {
        let (cmd, _rx) = apply_cmd(1, 1);
        let sent = cmd_tx.send(cmd).await;
        assert!(sent.is_ok(), "the worker is running");
        wait_event(&events, "apply [1,1]").await;
        let (cmd, _rx) = apply_cmd(2, 2);
        let sent = cmd_tx.send(cmd).await;
        assert!(sent.is_ok(), "the worker is running");
        wait_event(&events, "read [2,2]").await;
    }

    tracing::info!("--- fail apply [1,1] without releasing read [2,2]");
    {
        drop(apply_1);
        let result = UTConfig::<()>::timeout(WAIT_TIMEOUT, recv_applied(&mut resp_rx)).await?;
        let error = result.expect_err("the failed apply sends a storage error");
        let error = error.downcast::<StorageError<UTConfig>>()?;
        let source = io::Error::other("injected apply 1 failure");
        let source = UTConfig::<()>::err_from_error(&source);
        let expected = StorageError::apply(log_id(1, 0, 1), source);
        assert_eq!(expected, error);
        let released = read_2.send(());
        assert_eq!(Err(()), released);
        let got = events.lock().unwrap().clone();
        let want = vec!["read [1,1]", "apply [1,1]", "read [2,2]"];
        assert_eq!(want, got);
    }
    Ok(())
}

/// A read-ahead error stays behind the earlier apply's successful notification.
#[test]
fn next_read_error_follows_apply_success() -> anyhow::Result<()> {
    UTConfig::<()>::run(next_read_error_follows_apply_success_async())
}

async fn next_read_error_follows_apply_success_async() -> anyhow::Result<()> {
    let sm = RecordingStateMachine::default();
    let events = sm.events.clone();
    let gates = sm.gates.clone();
    let (cmd_tx, cmd_rx) = UTConfig::<()>::mpsc(16);
    let (resp_tx, mut resp_rx) = UTConfig::<()>::mpsc(16);
    let apply_1 = gates.gate("apply", 1);
    gates.fail("read", 2);
    let _handle = spawn_worker(sm, cmd_tx.clone(), cmd_rx, resp_tx);

    tracing::info!("--- finish the failing read [2,2] while apply [1,1] stays held");
    {
        let (cmd, _rx) = apply_cmd(1, 1);
        let sent = cmd_tx.send(cmd).await;
        assert!(sent.is_ok(), "the worker is running");
        wait_event(&events, "apply [1,1]").await;
        let (cmd, _rx) = apply_cmd(2, 2);
        let sent = cmd_tx.send(cmd).await;
        assert!(sent.is_ok(), "the worker is running");
        wait_event(&events, "read [2,2]").await;
        let mut response = Box::pin(resp_rx.recv());
        let polled = futures_util::poll!(response.as_mut());
        let pending = polled.is_pending();
        assert!(pending, "the later read error cannot overtake the held apply");
    }

    tracing::info!("--- release apply [1,1] and receive its success before the read [2,2] error");
    {
        drop(apply_1);
        let applied = UTConfig::<()>::timeout(WAIT_TIMEOUT, recv_applied(&mut resp_rx)).await??;
        assert_eq!((1, 2, log_id(1, 0, 1)), applied);
        let result = UTConfig::<()>::timeout(WAIT_TIMEOUT, recv_applied(&mut resp_rx)).await?;
        let error = result.expect_err("the failed read reaches the following apply");
        let error = error.downcast::<StorageError<UTConfig>>()?;
        let source = io::Error::other("injected read 2 failure");
        let source = UTConfig::<()>::err_from_error(&source);
        let expected = StorageError::apply(log_id(1, 0, 2), source);
        assert_eq!(expected, error);
        let got = events.lock().unwrap().clone();
        let want = vec!["read [1,1]", "apply [1,1]", "read [2,2]", "done [1,1]"];
        assert_eq!(want, got);
    }
    Ok(())
}

/// A full notification channel still lets the concurrent read start, then both bounded queues
/// drain in command order.
#[test]
fn full_notification_channel_keeps_read_progress() -> anyhow::Result<()> {
    UTConfig::<()>::run(full_notification_channel_keeps_read_progress_async())
}

async fn full_notification_channel_keeps_read_progress_async() -> anyhow::Result<()> {
    let sm = RecordingStateMachine::default();
    let events = sm.events.clone();
    let gates = sm.gates.clone();
    let (cmd_tx, cmd_rx) = UTConfig::<()>::mpsc(1);
    let (resp_tx, mut resp_rx) = UTConfig::<()>::mpsc(1);
    let read_2 = gates.gate("read", 2);
    let apply_1 = gates.gate("apply", 1);

    tracing::info!("--- fill the only notification slot before starting the worker");
    {
        let applied = ApplyResult {
            since: 0,
            end: 1,
            last_applied: log_id(1, 0, 0),
        };
        let response = CommandResult::new(Ok(Response::Apply(applied)));
        let notification = Notification::sm(response);
        let sent = resp_tx.send(notification).await;
        assert!(sent.is_ok(), "the notification slot is empty");
    }
    let _handle = spawn_worker(sm, cmd_tx.clone(), cmd_rx, resp_tx);

    tracing::info!("--- start read [2,2] even though apply [1,1] will need notification capacity");
    {
        let (cmd, _rx) = apply_cmd(1, 1);
        let sent = cmd_tx.send(cmd).await;
        assert!(sent.is_ok(), "the worker is running");
        wait_event(&events, "apply [1,1]").await;
        let (cmd, _rx) = apply_cmd(2, 2);
        let sent = cmd_tx.send(cmd).await;
        assert!(sent.is_ok(), "the worker is running");
        wait_event(&events, "read [2,2]").await;
        drop(apply_1);
        wait_event(&events, "done [1,1]").await;
        let cmd = record_func("func");
        let sent = cmd_tx.send(cmd).await;
        assert!(sent.is_ok(), "the worker is running");
    }

    tracing::info!("--- fill the command channel and drain the earlier notification while read [2,2] stays held");
    let (cmd, _rx) = apply_cmd(3, 3);
    let send = cmd_tx.send(cmd);
    let sent = UTConfig::<()>::timeout(WAIT_TIMEOUT, send).await?;
    let accepted = sent.is_ok();
    assert!(accepted, "the worker retains the function and frees command capacity");
    let cmd = record_func("tail_func");
    let mut send = Box::pin(cmd_tx.send(cmd));
    {
        let polled = futures_util::poll!(send.as_mut());
        let pending = polled.is_pending();
        assert!(pending, "apply [3,3] occupies the only command slot");
        let applied = UTConfig::<()>::timeout(WAIT_TIMEOUT, recv_applied(&mut resp_rx)).await??;
        assert_eq!((0, 1, log_id(1, 0, 0)), applied);
        let applied = UTConfig::<()>::timeout(WAIT_TIMEOUT, recv_applied(&mut resp_rx)).await??;
        assert_eq!((1, 2, log_id(1, 0, 1)), applied);
    }

    tracing::info!("--- release read [2,2] and finish every command");
    {
        drop(read_2);
        let sent = UTConfig::<()>::timeout(WAIT_TIMEOUT, send).await?;
        assert!(sent.is_ok(), "the worker frees command capacity");
        let applied = UTConfig::<()>::timeout(WAIT_TIMEOUT, recv_applied(&mut resp_rx)).await??;
        assert_eq!((2, 3, log_id(1, 0, 2)), applied);
        let applied = UTConfig::<()>::timeout(WAIT_TIMEOUT, recv_applied(&mut resp_rx)).await??;
        assert_eq!((3, 4, log_id(1, 0, 3)), applied);
        wait_event(&events, "tail_func").await;
        let got = events.lock().unwrap().clone();
        let want = vec![
            "read [1,1]",
            "apply [1,1]",
            "read [2,2]",
            "done [1,1]",
            "apply [2,2]",
            "done [2,2]",
            "func",
            "read [3,3]",
            "apply [3,3]",
            "done [3,3]",
            "tail_func",
        ];
        assert_eq!(want, got);
    }
    Ok(())
}

#[test]
fn late_commands_continue_prefetch_before_apply_finishes() -> anyhow::Result<()> {
    UTConfig::<()>::run(late_commands_continue_prefetch_before_apply_finishes_async())
}

async fn late_commands_continue_prefetch_before_apply_finishes_async() -> anyhow::Result<()> {
    let sm = RecordingStateMachine::default();
    let events = sm.events.clone();
    let gates = sm.gates.clone();
    let (cmd_tx, cmd_rx) = UTConfig::<()>::mpsc(16);
    let (resp_tx, mut resp_rx) = UTConfig::<()>::mpsc(16);
    let apply_1 = gates.gate("apply", 1);
    let read_2 = gates.gate("read", 2);
    let read_4 = gates.gate("read", 4);
    let _handle = spawn_worker(sm, cmd_tx.clone(), cmd_rx, resp_tx);
    let mut clients = Vec::new();

    tracing::info!("--- start an apply without an initial prefetch, then admit work during a held read");
    {
        let (command, client) = apply_cmd(1, 1);
        clients.push(client);
        let sent = cmd_tx.send(command).await;
        let accepted = sent.is_ok();
        assert!(accepted);
        wait_event(&events, "apply [1,1]").await;
        let (command, client) = apply_cmd(2, 2);
        clients.push(client);
        let sent = cmd_tx.send(command).await;
        let accepted = sent.is_ok();
        assert!(accepted);
        wait_event(&events, "read [2,2]").await;
        let (command, client) = apply_cmd(3, 3);
        clients.push(client);
        let sent = cmd_tx.send(command).await;
        let accepted = sent.is_ok();
        assert!(accepted);
        drop(read_2);
        wait_event(&events, "read [3,3]").await;
    }

    tracing::info!("--- the next read proves B and C were enqueued before the original apply finishes");
    {
        let (command, client) = apply_cmd(4, 4);
        clients.push(client);
        let sent = cmd_tx.send(command).await;
        let accepted = sent.is_ok();
        assert!(accepted);
        wait_event(&events, "read [4,4]").await;
        let (tail, client) = apply_cmd(5, 5);
        clients.push(client);
        for command in [record_func("func"), tail] {
            let sent = cmd_tx.send(command).await;
            let accepted = sent.is_ok();
            assert!(accepted);
        }
        let actual = event_snapshot(&events);
        let expected = vec!["read [1,1]", "apply [1,1]", "read [2,2]", "read [3,3]", "read [4,4]"];
        assert_eq!(expected, actual);
        drop(apply_1);
        let first = recv_applied(&mut resp_rx).await?;
        let last = log_id(1, 0, 1);
        let expected = (1, 2, last);
        assert_eq!(expected, first);
        let next = recv_applied(&mut resp_rx).await?;
        let last = log_id(1, 0, 3);
        let expected = (2, 4, last);
        assert_eq!(expected, next);
    }

    tracing::info!("--- release the last read and verify the barrier and every client response");
    {
        drop(read_4);
        let fourth = recv_applied(&mut resp_rx).await?;
        let last = log_id(1, 0, 4);
        let expected = (4, 5, last);
        assert_eq!(expected, fourth);
        let fifth = recv_applied(&mut resp_rx).await?;
        let last = log_id(1, 0, 5);
        let expected = (5, 6, last);
        assert_eq!(expected, fifth);
        let actual = event_snapshot(&events);
        let expected = vec![
            "read [1,1]",
            "apply [1,1]",
            "read [2,2]",
            "read [3,3]",
            "read [4,4]",
            "done [1,1]",
            "apply [2,3]",
            "done [2,3]",
            "apply [4,4]",
            "done [4,4]",
            "func",
            "read [5,5]",
            "apply [5,5]",
            "done [5,5]",
        ];
        assert_eq!(expected, actual);
        let mut responses = Vec::new();
        for client in clients {
            let response = client.await??;
            responses.push(response.log_id);
        }
        let expected = vec![
            log_id(1, 0, 1),
            log_id(1, 0, 2),
            log_id(1, 0, 3),
            log_id(1, 0, 4),
            log_id(1, 0, 5),
        ];
        assert_eq!(expected, responses);
    }
    Ok(())
}

#[test]
fn prefetch_cap_bounds_loaded_and_reading_entries() -> anyhow::Result<()> {
    let test = prefetch_cap_bounds_loaded_and_reading_entries_async();
    UTConfig::<()>::run(test)
}

async fn prefetch_cap_bounds_loaded_and_reading_entries_async() -> anyhow::Result<()> {
    let sm = RecordingStateMachine::default();
    let events = sm.events.clone();
    let gates = sm.gates.clone();
    let (cmd_tx, cmd_rx) = UTConfig::<()>::mpsc(16);
    let (resp_tx, mut resp_rx) = UTConfig::<()>::mpsc(16);
    let apply_1 = gates.gate("apply", 1);
    let apply_2 = gates.gate("apply", 2);
    let read_514 = gates.gate("read", 514);
    let read_tail = gates.gate("read", 1026);
    let _handle = spawn_worker(sm, cmd_tx.clone(), cmd_rx, resp_tx);

    tracing::info!("--- reserve 512 loaded plus 512 reading entries while the current apply remains held");
    {
        let (command, _client) = apply_cmd(1, 1);
        let sent = cmd_tx.send(command).await;
        let accepted = sent.is_ok();
        assert!(accepted);
        wait_event(&events, "apply [1,1]").await;
        let (command, _client) = apply_cmd(2, 513);
        let sent = cmd_tx.send(command).await;
        let accepted = sent.is_ok();
        assert!(accepted);
        wait_event(&events, "read [2,513]").await;
        let (command, _client) = apply_cmd(514, 1025);
        let sent = cmd_tx.send(command).await;
        let accepted = sent.is_ok();
        assert!(accepted);
        wait_event(&events, "read [514,1025]").await;
        let (command, _client) = apply_cmd(1026, 1026);
        let sent = cmd_tx.send(command).await;
        let accepted = sent.is_ok();
        assert!(accepted);
        let actual = event_snapshot(&events);
        let expected = vec!["read [1,1]", "apply [1,1]", "read [2,513]", "read [514,1025]"];
        assert_eq!(expected, actual);
    }

    tracing::info!("--- starting B returns credits without waiting for the pending read of C");
    {
        drop(apply_1);
        let first = recv_applied(&mut resp_rx).await?;
        let last = log_id(1, 0, 1);
        let expected = (1, 2, last);
        assert_eq!(expected, first);
        wait_event(&events, "apply [2,513]").await;
        drop(read_514);
        wait_event(&events, "read [1026,1026]").await;
        drop(apply_2);
        let second = recv_applied(&mut resp_rx).await?;
        let last = log_id(1, 0, 513);
        let expected = (2, 514, last);
        assert_eq!(expected, second);
        let third = recv_applied(&mut resp_rx).await?;
        let last = log_id(1, 0, 1025);
        let expected = (514, 1026, last);
        assert_eq!(expected, third);
        drop(read_tail);
        let tail = recv_applied(&mut resp_rx).await?;
        let last = log_id(1, 0, 1026);
        let expected = (1026, 1027, last);
        assert_eq!(expected, tail);
        let actual = event_snapshot(&events);
        let expected = vec![
            "read [1,1]",
            "apply [1,1]",
            "read [2,513]",
            "read [514,1025]",
            "done [1,1]",
            "apply [2,513]",
            "read [1026,1026]",
            "done [2,513]",
            "apply [514,1025]",
            "done [514,1025]",
            "apply [1026,1026]",
            "done [1026,1026]",
        ];
        assert_eq!(expected, actual);
    }
    Ok(())
}

#[test]
fn oversized_prefetch_command_waits_without_splitting_log_ids() -> anyhow::Result<()> {
    UTConfig::<()>::run(oversized_prefetch_command_waits_without_splitting_log_ids_async())
}

async fn oversized_prefetch_command_waits_without_splitting_log_ids_async() -> anyhow::Result<()> {
    let sm = RecordingStateMachine::default();
    let events = sm.events.clone();
    let gates = sm.gates.clone();
    let (cmd_tx, cmd_rx) = UTConfig::<()>::mpsc(1);
    let (resp_tx, mut resp_rx) = UTConfig::<()>::mpsc(16);
    let apply_1 = gates.gate("apply", 1);
    let _handle = spawn_worker(sm, cmd_tx.clone(), cmd_rx, resp_tx);

    tracing::info!("--- put an oversized command into the only command-channel slot during a held apply");
    {
        let (cmd, _client) = apply_cmd(1, 1);
        let sent = cmd_tx.send(cmd).await;
        let accepted = sent.is_ok();
        assert!(accepted, "the worker is running");
        wait_event(&events, "apply [1,1]").await;
        let (cmd, _client) = apply_cmd(2, 1026);
        let sent = cmd_tx.send(cmd).await;
        let accepted = sent.is_ok();
        assert!(accepted, "the worker is running");
    }

    tracing::info!("--- peeking frees command capacity while the oversized range stays unread");
    {
        let command = record_func("func");
        let sending = cmd_tx.send(command);
        let sent = UTConfig::<()>::timeout(WAIT_TIMEOUT, sending).await?;
        let accepted = sent.is_ok();
        assert!(accepted, "peeking frees one channel slot");
        let actual = event_snapshot(&events);
        let expected = vec!["read [1,1]", "apply [1,1]"];
        assert_eq!(expected, actual);
        drop(apply_1);
        let first = recv_applied(&mut resp_rx).await?;
        let expected = (1, 2, log_id(1, 0, 1));
        assert_eq!(expected, first);
        let next = recv_applied(&mut resp_rx).await?;
        let expected = (2, 1027, log_id(1, 0, 1026));
        assert_eq!(expected, next);
        wait_event(&events, "func").await;
        let actual = event_snapshot(&events);
        let expected = vec![
            "read [1,1]",
            "apply [1,1]",
            "done [1,1]",
            "read [2,1026]",
            "apply [2,1026]",
            "done [2,1026]",
            "func",
        ];
        assert_eq!(expected, actual);
    }
    Ok(())
}

#[test]
fn snapshot_command_stops_late_prefetch() -> anyhow::Result<()> {
    UTConfig::<()>::run(snapshot_command_stops_late_prefetch_async())
}

async fn snapshot_command_stops_late_prefetch_async() -> anyhow::Result<()> {
    let sm = RecordingStateMachine::default();
    let events = sm.events.clone();
    let gates = sm.gates.clone();
    let (cmd_tx, cmd_rx) = UTConfig::<()>::mpsc(16);
    let (resp_tx, mut resp_rx) = UTConfig::<()>::mpsc(16);
    let apply_1 = gates.gate("apply", 1);
    let _handle = spawn_worker(sm, cmd_tx.clone(), cmd_rx, resp_tx);

    tracing::info!("--- queue a snapshot between late apply commands while the first apply remains held");
    let (snapshot_tx, snapshot_rx) = UTConfig::<()>::oneshot();
    {
        let (cmd, _client) = apply_cmd(1, 1);
        let sent = cmd_tx.send(cmd).await;
        let accepted = sent.is_ok();
        assert!(accepted, "the worker is running");
        wait_event(&events, "apply [1,1]").await;
        let (next, _client) = apply_cmd(2, 2);
        let (last, _client) = apply_cmd(3, 3);
        for cmd in [next, Command::get_snapshot(snapshot_tx), last] {
            let sent = cmd_tx.send(cmd).await;
            let accepted = sent.is_ok();
            assert!(accepted, "the worker is running");
        }
        wait_event(&events, "read [2,2]").await;
        drop(apply_1);
    }

    tracing::info!("--- finish the prefetched prefix, execute the snapshot, and only then read its successor");
    {
        let mut applied = Vec::new();
        for _ in 0..3 {
            let result = recv_applied(&mut resp_rx).await?;
            applied.push(result);
        }
        let expected = vec![
            (1, 2, log_id(1, 0, 1)),
            (2, 3, log_id(1, 0, 2)),
            (3, 4, log_id(1, 0, 3)),
        ];
        assert_eq!(expected, applied);
        let snapshot = snapshot_rx.await?;
        let absent = snapshot.is_none();
        assert!(absent);
        let actual = event_snapshot(&events);
        let expected = vec![
            "read [1,1]",
            "apply [1,1]",
            "read [2,2]",
            "done [1,1]",
            "apply [2,2]",
            "done [2,2]",
            "snapshot",
            "read [3,3]",
            "apply [3,3]",
            "done [3,3]",
        ];
        assert_eq!(expected, actual);
    }
    Ok(())
}

#[test]
fn closed_command_channel_finishes_the_current_apply() -> anyhow::Result<()> {
    UTConfig::<()>::run(closed_command_channel_finishes_the_current_apply_async())
}

async fn closed_command_channel_finishes_the_current_apply_async() -> anyhow::Result<()> {
    let sm = RecordingStateMachine::default();
    let events = sm.events.clone();
    let gates = sm.gates.clone();
    let (cmd_tx, cmd_rx) = UTConfig::<()>::mpsc(1);
    let (resp_tx, mut resp_rx) = UTConfig::<()>::mpsc(1);
    let apply_1 = gates.gate("apply", 1);
    let handle = spawn_worker(sm, cmd_tx.clone(), cmd_rx, resp_tx);

    tracing::info!("--- close the command channel while the current apply remains held and no prefetch is available");
    {
        let (cmd, _client) = apply_cmd(1, 1);
        let sent = cmd_tx.send(cmd).await;
        let accepted = sent.is_ok();
        assert!(accepted, "the worker is running");
        wait_event(&events, "apply [1,1]").await;
        drop(cmd_tx);
        drop(handle);
        drop(apply_1);
    }

    tracing::info!("--- publish the final apply before the worker closes its notification channel");
    {
        let receiving = recv_applied(&mut resp_rx);
        let applied = UTConfig::<()>::timeout(WAIT_TIMEOUT, receiving).await??;
        let expected = (1, 2, log_id(1, 0, 1));
        assert_eq!(expected, applied);
        let receiving = resp_rx.recv();
        let next = UTConfig::<()>::timeout(WAIT_TIMEOUT, receiving).await?;
        let closed = next.is_none();
        assert!(closed);
        let actual = event_snapshot(&events);
        let expected = vec!["read [1,1]", "apply [1,1]", "done [1,1]"];
        assert_eq!(expected, actual);
    }
    Ok(())
}

#[test]
fn ready_prefix_applies_while_next_read_is_pending() -> anyhow::Result<()> {
    let test = ready_prefix_applies_while_next_read_is_pending_async();
    UTConfig::<()>::run(test)
}

async fn ready_prefix_applies_while_next_read_is_pending_async() -> anyhow::Result<()> {
    let sm = RecordingStateMachine::default();
    let events = sm.events.clone();
    let gates = sm.gates.clone();
    let (cmd_tx, cmd_rx) = UTConfig::<()>::mpsc(16);
    let (resp_tx, mut resp_rx) = UTConfig::<()>::mpsc(1);
    let apply_1 = gates.gate("apply", 1);
    let apply_2 = gates.gate("apply", 2);
    let read_3 = gates.gate("read", 3);

    tracing::info!("--- fill notification capacity, then hold A applying with B loaded and C reading");
    {
        let last_applied = log_id(1, 0, 0);
        let applied = ApplyResult {
            since: 0,
            end: 1,
            last_applied,
        };
        let result = CommandResult::new(Ok(Response::Apply(applied)));
        let notification = Notification::sm(result);
        let sent = resp_tx.send(notification).await;
        let accepted = sent.is_ok();
        assert!(accepted, "notification capacity is available");
    }
    let _handle = spawn_worker(sm, cmd_tx.clone(), cmd_rx, resp_tx);
    let clients = {
        let mut clients = Vec::new();
        let (command, client) = apply_cmd(1, 1);
        clients.push(client);
        let sent = cmd_tx.send(command).await;
        let accepted = sent.is_ok();
        assert!(accepted, "the worker is running");
        wait_event(&events, "apply [1,1]").await;
        let (command, client) = apply_cmd(2, 2);
        clients.push(client);
        let sent = cmd_tx.send(command).await;
        let accepted = sent.is_ok();
        assert!(accepted, "the worker is running");
        wait_event(&events, "read [2,2]").await;
        let (command, client) = apply_cmd(3, 3);
        clients.push(client);
        let sent = cmd_tx.send(command).await;
        let accepted = sent.is_ok();
        assert!(accepted, "the worker is running");
        wait_event(&events, "read [3,3]").await;
        let command = record_func("func");
        let sent = cmd_tx.send(command).await;
        let accepted = sent.is_ok();
        assert!(accepted, "the worker is running");
        let (command, client) = apply_cmd(4, 4);
        clients.push(client);
        let sent = cmd_tx.send(command).await;
        let accepted = sent.is_ok();
        assert!(accepted, "the worker is running");
        clients
    };

    tracing::info!("--- publishing A releases the SM for B while the generic read of C stays held");
    {
        drop(apply_1);
        wait_event(&events, "done [1,1]").await;
        let actual = event_snapshot(&events);
        let expected = vec!["read [1,1]", "apply [1,1]", "read [2,2]", "read [3,3]", "done [1,1]"];
        assert_eq!(expected, actual);
        let dummy = recv_applied(&mut resp_rx).await?;
        let expected_last = log_id(1, 0, 0);
        let expected = (0, 1, expected_last);
        assert_eq!(expected, dummy);
        let first = recv_applied(&mut resp_rx).await?;
        let expected_last = log_id(1, 0, 1);
        let expected = (1, 2, expected_last);
        assert_eq!(expected, first);
        wait_event(&events, "apply [2,2]").await;
        drop(apply_2);
        let second = recv_applied(&mut resp_rx).await?;
        let expected_last = log_id(1, 0, 2);
        let expected = (2, 3, expected_last);
        assert_eq!(expected, second);
        let actual = event_snapshot(&events);
        let expected = vec![
            "read [1,1]",
            "apply [1,1]",
            "read [2,2]",
            "read [3,3]",
            "done [1,1]",
            "apply [2,2]",
            "done [2,2]",
        ];
        assert_eq!(expected, actual);
    }

    tracing::info!("--- C completes before the FIFO function barrier permits the final read");
    {
        drop(read_3);
        let third = recv_applied(&mut resp_rx).await?;
        let expected_last = log_id(1, 0, 3);
        let expected = (3, 4, expected_last);
        assert_eq!(expected, third);
        let last = recv_applied(&mut resp_rx).await?;
        let expected_last = log_id(1, 0, 4);
        let expected = (4, 5, expected_last);
        assert_eq!(expected, last);
        let actual = event_snapshot(&events);
        let expected = vec![
            "read [1,1]",
            "apply [1,1]",
            "read [2,2]",
            "read [3,3]",
            "done [1,1]",
            "apply [2,2]",
            "done [2,2]",
            "apply [3,3]",
            "done [3,3]",
            "func",
            "read [4,4]",
            "apply [4,4]",
            "done [4,4]",
        ];
        assert_eq!(expected, actual);
        let mut responses = Vec::new();
        for client in clients {
            let response = client.await??;
            responses.push(response.log_id);
        }
        let expected = vec![log_id(1, 0, 1), log_id(1, 0, 2), log_id(1, 0, 3), log_id(1, 0, 4)];
        assert_eq!(expected, responses);
    }
    Ok(())
}

#[test]
fn queued_loaded_prefix_coalesces_before_barrier() -> anyhow::Result<()> {
    let test = queued_loaded_prefix_coalesces_before_barrier_async();
    UTConfig::<()>::run(test)
}

async fn queued_loaded_prefix_coalesces_before_barrier_async() -> anyhow::Result<()> {
    let sm = RecordingStateMachine::default();
    let events = sm.events.clone();
    let (loaded_tx, loaded_rx) = UTConfig::<()>::mpsc(4);
    let (resp_tx, mut resp_rx) = UTConfig::<()>::mpsc(16);
    let (progress_tx, progress_rx) = UTConfig::<()>::watch_channel(Progress::default());

    tracing::info!(
        "--- queue two loaded batches, a function barrier, and a separate loaded tail before starting the applier"
    );
    {
        for index in [2, 3] {
            let first = log_id(1, 0, index);
            let last = first;
            let entry = EntryOf::<UTConfig>::new_blank(first);
            let mut batch = ApplyBatch::new(first, last, Vec::new());
            batch.entries.push(Ok(entry));
            let sent = loaded_tx.send(Work::Apply(batch)).await;
            let accepted = sent.is_ok();
            assert!(accepted);
        }
        let (finished, acknowledged) = UTConfig::<()>::oneshot();
        let command = record_func("func");
        let barrier = Work::Barrier { command, finished };
        let sent = loaded_tx.send(barrier).await;
        let accepted = sent.is_ok();
        assert!(accepted);
        let first = log_id(1, 0, 4);
        let last = first;
        let entry = EntryOf::<UTConfig>::new_blank(first);
        let mut batch = ApplyBatch::new(first, last, Vec::new());
        batch.entries.push(Ok(entry));
        let sent = loaded_tx.send(Work::Apply(batch)).await;
        let accepted = sent.is_ok();
        assert!(accepted);
        drop(loaded_tx);

        let worker = Worker {
            id: 0,
            state_machine: sm,
            resp_tx,
        };
        let applying = worker.apply_loop(loaded_rx, progress_tx);
        let handle = UTConfig::<()>::spawn(applying);
        let merged = recv_applied(&mut resp_rx).await?;
        let expected_last = log_id(1, 0, 3);
        let expected = (2, 4, expected_last);
        assert_eq!(expected, merged);
        let tail = recv_applied(&mut resp_rx).await?;
        let expected_last = log_id(1, 0, 4);
        let expected = (4, 5, expected_last);
        assert_eq!(expected, tail);
        let acknowledged = acknowledged.await;
        assert_eq!(Ok(()), acknowledged);
        let joined = handle.await?;
        assert_eq!(Ok(()), joined);
        let actual = event_snapshot(&events);
        let expected = vec!["apply [2,3]", "done [2,3]", "func", "apply [4,4]", "done [4,4]"];
        assert_eq!(expected, actual);
        let actual = *progress_rx.borrow_watched();
        let expected = Progress {
            started: 3,
            finished: 3,
        };
        assert_eq!(expected, actual);
    }
    Ok(())
}

fn event_snapshot(events: &Events) -> Vec<String> {
    let events = events.lock().unwrap();
    events.clone()
}

#[test]
fn intake_limits_admitted_entries_until_apply_starts() -> anyhow::Result<()> {
    let test = intake_limits_admitted_entries_until_apply_starts_async();
    UTConfig::<()>::run(test)
}

async fn intake_limits_admitted_entries_until_apply_starts_async() -> anyhow::Result<()> {
    let (commands, incoming) = UTConfig::<()>::mpsc(4);
    let (output, mut prepared) = UTConfig::<()>::mpsc(4);
    let (progress, observed) = UTConfig::<()>::watch_channel(Progress::default());
    for (first, last) in [(1, 512), (513, 1024), (1025, 2048), (2049, 3072)] {
        let (command, _client) = apply_cmd(first, last);
        let sent = commands.send(command).await;
        let accepted = sent.is_ok();
        assert!(accepted);
    }
    let incoming = CommandReceiver::<UTConfig, RecordingStateMachine>::new(incoming, false);
    let intake = Intake::new(incoming, output, observed);
    let mut intake = Box::pin(intake.run());

    tracing::info!("--- drive intake until 1024 entries fill its reservation, without any reader or applier polling");
    {
        let polled = futures_util::poll!(intake.as_mut());
        let pending = polled.is_pending();
        assert!(pending);
        let batch = recv_unread_batch(&mut prepared);
        let first = log_id(1, 0, 1);
        let last = log_id(1, 0, 1024);
        let expected = (first, last);
        let actual = (batch.first, batch.last);
        assert_eq!(expected, actual);
        let responders = batch.client_resp_channels.into_iter();
        let indexes: Vec<_> = responders.map(|(index, _)| index).collect();
        let expected = vec![512, 1024];
        assert_eq!(expected, indexes);
        let empty = batch.entries.is_empty();
        assert!(empty, "intake reserves metadata without reading entries");
        let next = prepared.try_recv();
        let error = next.err();
        assert_eq!(Some(TryRecvError::Empty), error);
    }

    tracing::info!("--- starting an apply returns credits even while its completion and notification remain pending");
    {
        let started = Progress {
            started: 1024,
            finished: 0,
        };
        let sent = progress.send(started);
        assert_eq!(Ok(()), sent);
        let polled = futures_util::poll!(intake.as_mut());
        let pending = polled.is_pending();
        assert!(pending);
        let batch = recv_unread_batch(&mut prepared);
        let first = log_id(1, 0, 1025);
        let last = log_id(1, 0, 2048);
        let expected = (first, last);
        let actual = (batch.first, batch.last);
        assert_eq!(expected, actual);
        let next = prepared.try_recv();
        let error = next.err();
        assert_eq!(Some(TryRecvError::Empty), error);
    }

    tracing::info!("--- coalesced progress updates retain the absolute credit total, then EOF drains");
    {
        let started = Progress {
            started: 2048,
            finished: 1024,
        };
        let sent = progress.send(started);
        assert_eq!(Ok(()), sent);
        let finished = Progress {
            started: 2048,
            finished: 2048,
        };
        let sent = progress.send(finished);
        assert_eq!(Ok(()), sent);
        let polled = futures_util::poll!(intake.as_mut());
        let pending = polled.is_pending();
        assert!(pending);
        let batch = recv_unread_batch(&mut prepared);
        let first = log_id(1, 0, 2049);
        let last = log_id(1, 0, 3072);
        let expected = (first, last);
        let actual = (batch.first, batch.last);
        assert_eq!(expected, actual);
        drop(commands);
        let finished = intake.await;
        assert_eq!(Ok(()), finished);
        let next = prepared.try_recv();
        let error = next.err();
        assert_eq!(Some(TryRecvError::Disconnected), error);
    }
    Ok(())
}

fn recv_unread_batch(
    incoming: &mut MpscReceiverOf<UTConfig, Work<UTConfig, RecordingStateMachine>>,
) -> ApplyBatch<UTConfig> {
    let received = incoming.try_recv();
    let Ok(Work::Apply(batch)) = received else {
        panic!("the next admitted work is an apply batch");
    };
    batch
}
