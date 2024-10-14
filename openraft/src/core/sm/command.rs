use std::fmt::Debug;
use std::fmt::Formatter;

use crate::core::raft_msg::ResultSender;
use crate::display_ext::DisplaySlice;
use crate::error::Infallible;
use crate::log_id::RaftLogId;
use crate::type_config::alias::SnapshotDataOf;
use crate::RaftTypeConfig;
use crate::Snapshot;

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

    pub(crate) fn get_snapshot(tx: ResultSender<C, Option<Snapshot<C>>>) -> Self {
        let payload = CommandPayload::GetSnapshot { tx };
        Command::new(payload)
    }

    pub(crate) fn begin_receiving_snapshot(tx: ResultSender<C, Box<SnapshotDataOf<C>>, Infallible>) -> Self {
        let payload = CommandPayload::BeginReceivingSnapshot { tx };
        Command::new(payload)
    }

    pub(crate) fn install_full_snapshot(snapshot: Snapshot<C>) -> Self {
        let payload = CommandPayload::InstallFullSnapshot { snapshot };
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
    GetSnapshot {
        tx: ResultSender<C, Option<Snapshot<C>>>,
    },

    BeginReceivingSnapshot {
        tx: ResultSender<C, Box<SnapshotDataOf<C>>, Infallible>,
    },

    InstallFullSnapshot {
        snapshot: Snapshot<C>,
    },

    /// Apply the log entries to the state machine.
    Apply {
        entries: Vec<C::Entry>,
    },
}

impl<C> Debug for CommandPayload<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandPayload::BuildSnapshot => write!(f, "BuildSnapshot"),
            CommandPayload::GetSnapshot { .. } => write!(f, "GetSnapshot"),
            CommandPayload::InstallFullSnapshot { snapshot } => {
                write!(f, "InstallFullSnapshot: meta: {:?}", snapshot.meta)
            }
            CommandPayload::BeginReceivingSnapshot { .. } => {
                write!(f, "BeginReceivingSnapshot")
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
            (CommandPayload::BeginReceivingSnapshot { .. }, CommandPayload::BeginReceivingSnapshot { .. }) => true,
            (
                CommandPayload::InstallFullSnapshot { snapshot: s1 },
                CommandPayload::InstallFullSnapshot { snapshot: s2 },
            ) => s1.meta == s2.meta,
            (CommandPayload::Apply { entries: entries1 }, CommandPayload::Apply { entries: entries2 }) => {
                // Entry may not be `Eq`, we just compare log id.
                // This would be enough for testing.
                entries1.iter().map(|e| e.get_log_id().clone()).collect::<Vec<_>>()
                    == entries2.iter().map(|e| e.get_log_id().clone()).collect::<Vec<_>>()
            }
            _ => false,
        }
    }
}
