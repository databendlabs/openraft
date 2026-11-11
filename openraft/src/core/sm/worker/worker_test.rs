use std::io;
use std::ops::Bound;
use std::ops::RangeBounds;
use std::sync::Arc;
use std::sync::Mutex;

use futures_util::Stream;
use futures_util::TryStreamExt;

use crate::OptionalSend;
use crate::RaftLogReader;
use crate::RaftTypeConfig;
use crate::async_runtime::MpscReceiver;
use crate::async_runtime::MpscSender;
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
use crate::type_config::alias::OneshotReceiverOf;
use crate::type_config::alias::SnapshotMetaOf;
use crate::type_config::alias::SnapshotOf;
use crate::type_config::alias::StoredMembershipOf;

/// Records every `apply()` call as `apply [first,last]` and every `ExternalFunc` as `func`.
#[derive(Clone, Default)]
struct RecordingStateMachine {
    events: Arc<Mutex<Vec<String>>>,
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
        let event = format!("apply [{},{}]", indexes[0], indexes[indexes.len() - 1]);
        self.events.lock().unwrap().push(event);
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

/// Returns a blank entry of term 1 for every index in the requested range.
struct BlankLogReader;

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
        let func = Command::ExternalFunc {
            func: Box::new(|sm: &mut RecordingStateMachine| {
                let events = sm.events.clone();
                Box::pin(async move { events.lock().unwrap().push("func".to_string()) })
            }),
        };
        for cmd in [cmd_1, cmd_2, func, cmd_3] {
            let sent = cmd_tx.send(cmd).await;
            assert!(sent.is_ok(), "the channel has room for every command");
        }
        [rx_1, rx_2, rx_3]
    };

    tracing::info!("--- start the worker and collect its Apply responses");
    {
        let cmd_rx = CommandReceiver::new(cmd_rx, false);
        let span = tracing::Span::none();
        let _handle = Worker::spawn(0, sm, BlankLogReader, resp_tx, (cmd_tx, cmd_rx), span);

        let mut applied = Vec::new();
        for _ in 0..2 {
            let notification = resp_rx.recv().await.expect("the worker sends one response per batch");
            let Notification::StateMachine { command_result } = notification else {
                panic!("unexpected notification: {}", notification);
            };
            let Response::Apply(result) = command_result.result? else {
                panic!("unexpected state machine response");
            };
            applied.push((result.since, result.end, result.last_applied));
        }
        let want = vec![(1, 4, log_id(1, 0, 3)), (4, 5, log_id(1, 0, 4))];
        assert_eq!(want, applied);

        let got = events.lock().unwrap().clone();
        assert_eq!(vec!["apply [1,3]", "func", "apply [4,4]"], got);
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
