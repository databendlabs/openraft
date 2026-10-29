//! State machine control handle

use std::future::Future;

use crate::OptionalSend;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::async_runtime::MpscSender;
use crate::async_runtime::MpscWeakSender;
use crate::async_runtime::SendError;
use crate::core::sm;
use crate::storage::RaftStateMachine;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::JoinHandleOf;
use crate::type_config::alias::MpscSenderOf;
use crate::type_config::alias::MpscWeakSenderOf;
use crate::type_config::alias::SmSnapshotOf;

/// State machine worker handle for sending command to it.
pub(crate) struct Handle<C, SM = ()>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    pub(in crate::core::sm) cmd_tx: MpscSenderOf<C, sm::Command<C, SM>>,

    #[allow(dead_code)]
    pub(in crate::core::sm) join_handle: JoinHandleOf<C, ()>,
}

impl<C, SM> Handle<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    /// Returns a future that sends `cmd` to the worker.
    ///
    /// The future owns a clone of the sender and `cmd`, so it borrows nothing and can be retained
    /// while other work runs. Polling the same future keeps its place among the senders waiting
    /// for capacity. A failed send reports the storage error of the unsent command.
    pub(crate) fn send(
        &self,
        cmd: sm::Command<C, SM>,
    ) -> impl Future<Output = Result<(), StorageError<C>>> + OptionalSend + use<C, SM> {
        tracing::debug!("sending command to state machine worker: {:?}", cmd);
        let cmd_tx = self.cmd_tx.clone();
        async move {
            let res = cmd_tx.send(cmd).await;
            res.map_err(Self::send_error)
        }
    }

    /// Converts a failed send to the storage error of the unsent command.
    fn send_error(err: SendError<sm::Command<C, SM>>) -> StorageError<C> {
        let reason = err.to_string();
        match err.0 {
            sm::Command::Apply { last, .. } => StorageError::apply(last, C::err_from_string(reason)),
            _ => StorageError::write_state_machine(C::err_from_string("cannot send to sm::Worker")),
        }
    }

    /// Create a [`SnapshotReader`] to get the current snapshot from the state machine.
    pub(crate) fn new_snapshot_reader(&self) -> SnapshotReader<C, SM> {
        SnapshotReader {
            cmd_tx: self.cmd_tx.downgrade(),
        }
    }
}

/// A handle for retrieving a snapshot from the state machine.
pub(crate) struct SnapshotReader<C, SM = ()>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    /// Weak command sender to the state machine worker.
    ///
    /// It is weak because the [`Worker`] watches the close event of this channel for shutdown.
    ///
    /// [`Worker`]: sm::worker::Worker
    cmd_tx: MpscWeakSenderOf<C, sm::Command<C, SM>>,
}

impl<C, SM> SnapshotReader<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    /// Get a snapshot from the state machine.
    ///
    /// If the state machine worker has shutdown, it will return an error.
    /// If there is no snapshot available, it will return `Ok(None)`.
    pub(crate) async fn get_snapshot(&self) -> Result<Option<SmSnapshotOf<C, SM>>, &'static str> {
        let (tx, rx) = C::oneshot();

        let cmd = sm::Command::get_snapshot(tx);
        tracing::debug!("SnapshotReader sending command to sm::Worker: {:?}", cmd);

        let Some(cmd_tx) = self.cmd_tx.upgrade() else {
            tracing::info!("failed to upgrade cmd_tx, sm::Worker may have shutdown");
            return Err("failed to upgrade cmd_tx, sm::Worker may have shutdown");
        };

        // If fail to send command, cmd is dropped and tx will be dropped.
        cmd_tx.send(cmd).await.ok();

        let snapshot = match rx.await {
            Ok(x) => x,
            Err(_e) => {
                tracing::error!("failed to receive snapshot, sm::Worker may have shutdown");
                return Err("failed to receive snapshot, sm::Worker may have shutdown");
            }
        };

        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use futures_util::FutureExt;

    use crate::StorageError;
    use crate::async_runtime::MpscReceiver;
    use crate::async_runtime::TryRecvError;
    use crate::base::BoxFuture;
    use crate::core::sm;
    use crate::core::sm::handle::Handle;
    use crate::engine::testing::UTConfig;
    use crate::engine::testing::log_id;
    use crate::type_config::TypeConfigExt;

    type C = UTConfig;

    /// A send retained while the channel is full delivers its command exactly once, after the
    /// receiver frees a slot.
    #[test]
    fn retained_send_delivers_once_after_capacity_frees() {
        C::run(async {
            let (cmd_tx, mut cmd_rx) = C::mpsc(1);
            let handle: Handle<C> = Handle {
                cmd_tx,
                join_handle: C::spawn(async {}),
            };

            let sent = handle.send(sm::Command::build_snapshot()).now_or_never();
            assert_eq!(Some(Ok(())), sent);

            let mut send: BoxFuture<'static, Result<(), StorageError<C>>> =
                Box::pin(handle.send(sm::Command::apply(log_id(1, 1, 1), log_id(1, 1, 2), vec![])));
            for _ in 0..3 {
                let polled = send.as_mut().now_or_never();
                assert_eq!(None, polled, "the channel is full");
            }

            let received = cmd_rx.try_recv();
            assert_eq!(Ok(sm::Command::build_snapshot()), received);

            let polled = send.as_mut().now_or_never();
            assert_eq!(Some(Ok(())), polled);

            let received = cmd_rx.try_recv();
            let want = sm::Command::apply(log_id(1, 1, 1), log_id(1, 1, 2), vec![]);
            assert_eq!(Ok(want), received);

            let received = cmd_rx.try_recv();
            assert_eq!(Err(TryRecvError::Empty), received);
        });
    }

    #[test]
    fn closed_channel_reports_the_unsent_command() {
        C::run(async {
            let (cmd_tx, cmd_rx) = C::mpsc(1);
            drop(cmd_rx);
            let handle: Handle<C> = Handle {
                cmd_tx,
                join_handle: C::spawn(async {}),
            };

            let res = handle.send(sm::Command::apply(log_id(1, 1, 1), log_id(1, 1, 2), vec![])).await;
            let want = StorageError::apply(log_id(1, 1, 2), C::err_from_string("channel closed"));
            assert_eq!(Err(want), res);

            let res = handle.send(sm::Command::build_snapshot()).await;
            let want = StorageError::write_state_machine(C::err_from_string("cannot send to sm::Worker"));
            assert_eq!(Err(want), res);
        });
    }
}
