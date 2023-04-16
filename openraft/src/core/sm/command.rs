use std::fmt::Debug;
use std::fmt::Formatter;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use tokio::sync::oneshot;

use crate::display_ext::DisplaySlice;
use crate::error::InstallSnapshotError;
use crate::raft::InstallSnapshotRequest;
use crate::storage::RaftStateMachine;
use crate::RaftTypeConfig;
use crate::Snapshot;
use crate::SnapshotMeta;

pub(crate) struct Command<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    pub(crate) command_id: CommandSeq,
    pub(crate) payload: CommandPayload<C, SM>,

    /// Custom respond function to be called when the command is done.
    pub(crate) respond: Box<dyn FnOnce() + Send + 'static>,
}

impl<C, SM> Debug for Command<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateMachineCommand")
            .field("command_id", &self.command_id)
            .field("payload", &self.payload)
            .finish()
    }
}

impl<C, SM> Command<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    /// Generate the next command seq with atomic increment.
    fn next_seq() -> CommandSeq {
        static SEQ: AtomicU64 = AtomicU64::new(1);
        SEQ.fetch_add(1, Ordering::Relaxed)
    }

    pub(crate) fn new<F>(payload: CommandPayload<C, SM>, respond: F) -> Self
    where F: FnOnce() + Send + 'static {
        Self {
            command_id: Self::next_seq(),
            payload,
            respond: Box::new(respond),
        }
    }

    pub(crate) fn build_snapshot() -> Self {
        let payload = CommandPayload::BuildSnapshot;
        Command::new(payload, || {})
    }

    pub(crate) fn get_snapshot(tx: oneshot::Sender<Option<Snapshot<C::NodeId, C::Node, SM::SnapshotData>>>) -> Self {
        let payload = CommandPayload::GetSnapshot { tx };
        Command::new(payload, || {})
    }

    pub(crate) fn receive(
        req: InstallSnapshotRequest<C>,
        tx: oneshot::Sender<Result<(), InstallSnapshotError>>,
    ) -> Self {
        let payload = CommandPayload::ReceiveSnapshotChunk { req, tx };
        Command::new(payload, || {})
    }

    pub(crate) fn install_snapshot(snapshot_meta: SnapshotMeta<C::NodeId, C::Node>) -> Self {
        let payload = CommandPayload::FinalizeSnapshot {
            install: true,
            snapshot_meta,
        };
        Command::new(payload, || {})
    }

    pub(crate) fn cancel_snapshot(snapshot_meta: SnapshotMeta<C::NodeId, C::Node>) -> Self {
        let payload = CommandPayload::FinalizeSnapshot {
            install: false,
            snapshot_meta,
        };
        Command::new(payload, || {})
    }

    pub(crate) fn apply(entries: Vec<C::Entry>) -> Self {
        let payload = CommandPayload::Apply { entries };
        Command::new(payload, || {})
    }
}

/// A sequence number of a state machine command.
///
/// It is used to identify and consume a submitted command when the command callback is received by
/// RaftCore.
pub(crate) type CommandSeq = u64;

/// The payload of a state machine command.
pub(crate) enum CommandPayload<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    /// Instruct the state machine to create a snapshot based on its most recent view.
    BuildSnapshot,

    /// Get the latest built snapshot.
    GetSnapshot {
        tx: oneshot::Sender<Option<Snapshot<C::NodeId, C::Node, SM::SnapshotData>>>,
    },

    /// Receive a chunk of snapshot.
    ///
    /// If it is the final chunk, the snapshot stream will be closed and saved.
    ///
    /// Installing a snapshot includes two steps: ReceiveSnapshotChunk and FinalizeSnapshot.
    ReceiveSnapshotChunk {
        req: InstallSnapshotRequest<C>,
        tx: oneshot::Sender<Result<(), InstallSnapshotError>>,
    },

    /// After receiving all chunks, finalize the snapshot by installing it or discarding it,
    /// if the snapshot is stale(the snapshot last log id is smaller than the local committed).
    FinalizeSnapshot {
        /// To install it, or just discard it.
        install: bool,
        snapshot_meta: SnapshotMeta<C::NodeId, C::Node>,
    },

    /// Apply the log entries to the state machine.
    Apply { entries: Vec<C::Entry> },
}

impl<C, SM> Debug for CommandPayload<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandPayload::BuildSnapshot => write!(f, "BuildSnapshot"),
            CommandPayload::GetSnapshot { .. } => write!(f, "GetSnapshot"),
            CommandPayload::ReceiveSnapshotChunk { req, .. } => {
                write!(f, "ReceiveSnapshotChunk: {:?}", req)
            }
            CommandPayload::FinalizeSnapshot { install, snapshot_meta } => {
                write!(f, "FinalizeSnapshot: install:{} {:?}", install, snapshot_meta)
            }
            CommandPayload::Apply { entries } => write!(f, "Apply: {}", DisplaySlice::<_>(entries)),
        }
    }
}
