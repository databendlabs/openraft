//! State machine control handle

use tokio::sync::mpsc;

use crate::core::sm;
use crate::type_config::alias::JoinHandleOf;
use crate::type_config::TypeConfigExt;
use crate::RaftTypeConfig;
use crate::Snapshot;

/// State machine worker handle for sending command to it.
pub(crate) struct Handle<C>
where C: RaftTypeConfig
{
    pub(in crate::core::sm) cmd_tx: mpsc::UnboundedSender<sm::Command<C>>,

    #[allow(dead_code)]
    pub(in crate::core::sm) join_handle: JoinHandleOf<C, ()>,
}

impl<C> Handle<C>
where C: RaftTypeConfig
{
    pub(crate) fn send(&mut self, cmd: sm::Command<C>) -> Result<(), mpsc::error::SendError<sm::Command<C>>> {
        tracing::debug!("sending command to state machine worker: {:?}", cmd);
        self.cmd_tx.send(cmd)
    }

    /// Create a [`SnapshotReader`] to get the current snapshot from the state machine.
    pub(crate) fn new_snapshot_reader(&self) -> SnapshotReader<C> {
        SnapshotReader {
            cmd_tx: self.cmd_tx.downgrade(),
        }
    }
}

/// A handle for retrieving a snapshot from the state machine.
pub(crate) struct SnapshotReader<C>
where C: RaftTypeConfig
{
    /// Weak command sender to the state machine worker.
    ///
    /// It is weak because the [`Worker`] watches the close event of this channel for shutdown.
    ///
    /// [`Worker`]: sm::worker::Worker
    cmd_tx: mpsc::WeakUnboundedSender<sm::Command<C>>,
}

impl<C> SnapshotReader<C>
where C: RaftTypeConfig
{
    /// Get a snapshot from the state machine.
    ///
    /// If the state machine worker has shutdown, it will return an error.
    /// If there is not snapshot available, it will return `Ok(None)`.
    pub(crate) async fn get_snapshot(&self) -> Result<Option<Snapshot<C>>, &'static str> {
        let (tx, rx) = C::oneshot();

        let cmd = sm::Command::get_snapshot(tx);
        tracing::debug!("SnapshotReader sending command to sm::Worker: {:?}", cmd);

        let Some(cmd_tx) = self.cmd_tx.upgrade() else {
            tracing::info!("failed to upgrade cmd_tx, sm::Worker may have shutdown");
            return Err("failed to upgrade cmd_tx, sm::Worker may have shutdown");
        };

        // If fail to send command, cmd is dropped and tx will be dropped.
        let _ = cmd_tx.send(cmd);

        let got = match rx.await {
            Ok(x) => x,
            Err(_e) => {
                tracing::error!("failed to receive snapshot, sm::Worker may have shutdown");
                return Err("failed to receive snapshot, sm::Worker may have shutdown");
            }
        };

        // Safe unwrap(): error is Infallible.
        let snapshot = got.unwrap();

        Ok(snapshot)
    }
}
