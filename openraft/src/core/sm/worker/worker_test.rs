use std::collections::BTreeMap;
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
use crate::async_runtime::MpscReceiver;
use crate::async_runtime::MpscSender;
use crate::core::ApplyResult;
use crate::core::notification::Notification;
use crate::core::sm::Command;
use crate::core::sm::Response;
use crate::core::sm::command_receiver::CommandReceiver;
use crate::core::sm::worker::Worker;
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

/// Records `apply [first,last]` once `apply()` has consumed its entries, and `done [first,last]`
/// when it returns; `ExternalFunc` commands record `func`.
///
/// An `apply()` whose first index has a gate waits for the gate to be released before it returns.
#[derive(Clone, Default)]
struct RecordingStateMachine {
    events: Events,
    gates: Arc<Mutex<BTreeMap<u64, OneshotReceiverOf<UTConfig, ()>>>>,
}

impl RecordingStateMachine {
    /// Make the `apply()` call that starts at `index` wait until the returned sender is dropped.
    fn gate(&self, index: u64) -> OneshotSenderOf<UTConfig, ()> {
        let (tx, rx) = UTConfig::<()>::oneshot();
        self.gates.lock().unwrap().insert(index, rx);
        tx
    }
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

        let gate = self.gates.lock().unwrap().remove(&indexes[0]);
        if let Some(gate) = gate {
            let _released = gate.await;
        }
        self.events.lock().unwrap().push(format!("done {range}"));
        Ok(())
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {}

    async fn install_snapshot(&mut self, _meta: &SnapshotMetaOf<UTConfig>, _snapshot: ()) -> Result<(), io::Error> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    async fn get_current_snapshot(&mut self) -> Result<Option<SnapshotOf<UTConfig, ()>>, io::Error> {
        Ok(None)
    }
}

/// Returns a blank entry of term 1 for every index in the requested range, and records each
/// read as `read [first,last] by <name>`.
struct BlankLogReader {
    name: &'static str,
    events: Events,
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
        let event = format!("read [{},{}] by {}", start, end - 1, self.name);
        self.events.lock().unwrap().push(event);
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

/// Spawn a worker on `sm` with an `apply` and a `prefetch` reader that record into `sm.events`.
fn spawn_worker(
    sm: RecordingStateMachine,
    cmd_tx: MpscSenderOf<UTConfig, Command<UTConfig, RecordingStateMachine>>,
    cmd_rx: MpscReceiverOf<UTConfig, Command<UTConfig, RecordingStateMachine>>,
    resp_tx: MpscSenderOf<UTConfig, Notification<UTConfig>>,
) -> impl Sized {
    let reader = |name| BlankLogReader {
        name,
        events: sm.events.clone(),
    };
    let (log_reader, prefetch_reader) = (reader("apply"), reader("prefetch"));
    let cmd_rx = CommandReceiver::new(cmd_rx, false);
    let span = tracing::Span::none();
    Worker::spawn(0, sm, log_reader, prefetch_reader, resp_tx, (cmd_tx, cmd_rx), span)
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
            "read [1,3] by apply",
            "apply [1,3]",
            "done [1,3]",
            "func",
            "read [4,4] by apply",
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

/// While a batch is applying, the worker reads each arriving `Apply` with the prefetch reader and
/// applies them all as the next batch; a command of another kind is not read ahead and runs after
/// the batch.
#[test]
fn prefetch_next_apply_during_apply() -> anyhow::Result<()> {
    UTConfig::<()>::run(prefetch_next_apply_during_apply_async())
}

async fn prefetch_next_apply_during_apply_async() -> anyhow::Result<()> {
    let sm = RecordingStateMachine::default();
    let events = sm.events.clone();
    let (cmd_tx, cmd_rx) = UTConfig::<()>::mpsc(16);
    let (resp_tx, mut resp_rx) = UTConfig::<()>::mpsc(16);
    let gate_1 = sm.gate(1);
    let gate_4 = sm.gate(4);
    let _handle = spawn_worker(sm, cmd_tx.clone(), cmd_rx, resp_tx);

    tracing::info!("--- Apply [2,2] and [3,3] arriving while [1,1] applies are read ahead as one batch");
    {
        let (cmd, _rx) = apply_cmd(1, 1);
        let sent = cmd_tx.send(cmd).await;
        assert!(sent.is_ok(), "the worker is running");
        wait_event(&events, "apply [1,1]").await;

        for index in [2, 3] {
            let (cmd, _rx) = apply_cmd(index, index);
            let sent = cmd_tx.send(cmd).await;
            assert!(sent.is_ok(), "the worker is running");
            wait_event(&events, &format!("read [{index},{index}] by prefetch")).await;
        }
        let done = events.lock().unwrap().iter().any(|e| e == "done [1,1]");
        assert!(!done, "the reads overlap apply [1,1]");

        drop(gate_1);
        let applied = [recv_applied(&mut resp_rx).await?, recv_applied(&mut resp_rx).await?];
        let want = [(1, 2, log_id(1, 0, 1)), (2, 4, log_id(1, 0, 3))];
        assert_eq!(want, applied);
    }

    tracing::info!("--- a command arriving while [4,4] applies stops the prefetch and runs next");
    {
        let (cmd, _rx) = apply_cmd(4, 4);
        let sent = cmd_tx.send(cmd).await;
        assert!(sent.is_ok(), "the worker is running");
        wait_event(&events, "apply [4,4]").await;

        let (cmd, _rx) = apply_cmd(5, 5);
        for cmd in [record_func("func"), cmd] {
            let sent = cmd_tx.send(cmd).await;
            assert!(sent.is_ok(), "the worker is running");
        }

        drop(gate_4);
        let applied = [recv_applied(&mut resp_rx).await?, recv_applied(&mut resp_rx).await?];
        let want = [(4, 5, log_id(1, 0, 4)), (5, 6, log_id(1, 0, 5))];
        assert_eq!(want, applied);
    }

    let got = events.lock().unwrap().clone();
    let want = vec![
        "read [1,1] by apply",
        "apply [1,1]",
        "read [2,2] by prefetch",
        "read [3,3] by prefetch",
        "done [1,1]",
        "apply [2,3]",
        "done [2,3]",
        "read [4,4] by apply",
        "apply [4,4]",
        "done [4,4]",
        "func",
        "read [5,5] by apply",
        "apply [5,5]",
        "done [5,5]",
    ];
    assert_eq!(want, got);
    Ok(())
}
