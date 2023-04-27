use std::fmt::Debug;
use std::fmt::Formatter;

use tokio::sync::oneshot;

use crate::display_ext::DisplaySlice;
use crate::log_id::RaftLogId;
use crate::raft::InstallSnapshotRequest;
use crate::MessageSummary;
use crate::RaftTypeConfig;
use crate::Snapshot;
use crate::SnapshotMeta;

#[derive(PartialEq)]
pub(crate) struct Command<C>
where C: RaftTypeConfig
{
    pub(crate) seq: CommandSeq,
    pub(crate) payload: CommandPayload<C>,
}

impl<C> Debug for Command<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateMachineCommand")
            .field("seq", &self.seq)
            .field("payload", &self.payload)
            .finish()
    }
}

impl<C> Command<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(payload: CommandPayload<C>) -> Self {
        Self { seq: 0, payload }
    }

    #[allow(dead_code)]
    pub(crate) fn seq(&self) -> CommandSeq {
        self.seq
    }

    pub(crate) fn with_seq(mut self, seq: CommandSeq) -> Self {
        self.seq = seq;
        self
    }

    pub(crate) fn set_seq(&mut self, seq: CommandSeq) {
        self.seq = seq;
    }

    pub(crate) fn build_snapshot() -> Self {
        let payload = CommandPayload::BuildSnapshot;
        Command::new(payload)
    }

    pub(crate) fn get_snapshot(tx: oneshot::Sender<Option<Snapshot<C>>>) -> Self {
        let payload = CommandPayload::GetSnapshot { tx };
        Command::new(payload)
    }

    pub(crate) fn receive(req: InstallSnapshotRequest<C>) -> Self {
        let payload = CommandPayload::ReceiveSnapshotChunk { req };
        Command::new(payload)
    }

    // TODO: all sm command should have a command seq.
    pub(crate) fn install_snapshot(snapshot_meta: SnapshotMeta<C::NodeId, C::Node>) -> Self {
        let payload = CommandPayload::FinalizeSnapshot {
            install: true,
            snapshot_meta,
        };
        Command::new(payload)
    }

    pub(crate) fn cancel_snapshot(snapshot_meta: SnapshotMeta<C::NodeId, C::Node>) -> Self {
        let payload = CommandPayload::FinalizeSnapshot {
            install: false,
            snapshot_meta,
        };
        Command::new(payload)
    }

    pub(crate) fn apply(entries: Vec<C::Entry>) -> Self {
        let payload = CommandPayload::Apply { entries };
        Command::new(payload)
    }
}

// TODO: move to other mod, it is shared by log, sm and replication
/// A sequence number of a state machine command.
///
/// It is used to identify and consume a submitted command when the command callback is received by
/// RaftCore.
pub(crate) type CommandSeq = u64;

/// The payload of a state machine command.
pub(crate) enum CommandPayload<C>
where C: RaftTypeConfig
{
    /// Instruct the state machine to create a snapshot based on its most recent view.
    BuildSnapshot,

    /// Get the latest built snapshot.
    GetSnapshot { tx: oneshot::Sender<Option<Snapshot<C>>> },

    /// Receive a chunk of snapshot.
    ///
    /// If it is the final chunk, the snapshot stream will be closed and saved.
    ///
    /// Installing a snapshot includes two steps: ReceiveSnapshotChunk and FinalizeSnapshot.
    ReceiveSnapshotChunk { req: InstallSnapshotRequest<C> },

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

impl<C> Debug for CommandPayload<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandPayload::BuildSnapshot => write!(f, "BuildSnapshot"),
            CommandPayload::GetSnapshot { .. } => write!(f, "GetSnapshot"),
            CommandPayload::ReceiveSnapshotChunk { req, .. } => {
                write!(f, "ReceiveSnapshotChunk: {}", req.summary())
            }
            CommandPayload::FinalizeSnapshot { install, snapshot_meta } => {
                write!(f, "FinalizeSnapshot: install:{} {:?}", install, snapshot_meta)
            }
            CommandPayload::Apply { entries } => write!(f, "Apply: {}", DisplaySlice::<_>(entries)),
        }
    }
}

// `PartialEq` is only used for testing
impl<C> PartialEq for CommandPayload<C>
where C: RaftTypeConfig
{
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (CommandPayload::BuildSnapshot, CommandPayload::BuildSnapshot) => true,
            (CommandPayload::GetSnapshot { .. }, CommandPayload::GetSnapshot { .. }) => true,
            (
                CommandPayload::ReceiveSnapshotChunk { req: req1, .. },
                CommandPayload::ReceiveSnapshotChunk { req: req2, .. },
            ) => req1 == req2,
            (
                CommandPayload::FinalizeSnapshot {
                    install: install1,
                    snapshot_meta: meta1,
                },
                CommandPayload::FinalizeSnapshot {
                    install: install2,
                    snapshot_meta: meta2,
                },
            ) => install1 == install2 && meta1 == meta2,
            (CommandPayload::Apply { entries: entries1 }, CommandPayload::Apply { entries: entries2 }) => {
                // Entry may not be `Eq`, we just compare log id.
                // This would be enough for testing.
                entries1.iter().map(|e| *e.get_log_id()).collect::<Vec<_>>()
                    == entries2.iter().map(|e| *e.get_log_id()).collect::<Vec<_>>()
            }
            _ => false,
        }
    }
}
