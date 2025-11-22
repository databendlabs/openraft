//! Callbacks used by Storage API

use std::io;

use crate::RaftTypeConfig;
use crate::StorageError;
use crate::raft_state::IOId;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::OneshotSenderOf;
use crate::type_config::alias::WatchSenderOf;
use crate::type_config::async_runtime::oneshot::OneshotSender;
use crate::type_config::async_runtime::watch::WatchSender;

/// Type alias for log flush callback (deprecated).
#[deprecated(since = "0.10.0", note = "Use `IOFlushed` instead")]
pub type LogFlushed<C> = IOFlushed<C>;

/// A callback for completion of io operation to [`RaftLogStorage`].
///
/// [`RaftLogStorage`]: `crate::storage::RaftLogStorage`
pub enum IOFlushed<C>
where C: RaftTypeConfig
{
    /// A placeholder that does nothing when `io_completed` is called.
    ///
    /// Use this when an IO operation completes but no callback is expected.
    Noop,

    /// A callback that sends a notification through a channel when IO completes.
    Notify(IOFlushedNotify<C>),

    /// A simple signal that notifies completion with just the result status.
    ///
    /// This is used when only completion notification is needed without log ID
    /// or other detailed information. It propagates the IO result (success or error).
    Signal(OneshotSenderOf<C, Result<(), io::Error>>),
}

/// Inner struct holding the notification data for [`IOFlushed::Notify`].
///
/// This struct is public but has private fields, preventing external construction.
pub struct IOFlushedNotify<C>
where C: RaftTypeConfig
{
    /// The IO ID to send when the IO completes.
    io_id: IOId<C>,

    /// The channel to send the IO completion result through.
    tx: WatchSenderOf<C, Result<IOId<C>, StorageError<C>>>,
}

impl<C> IOFlushed<C>
where C: RaftTypeConfig
{
    /// Create a no-op callback that does nothing when `io_completed` is called.
    pub fn noop() -> Self {
        Self::Noop
    }

    /// Create a signal callback that sends the IO result when complete.
    ///
    /// This is used for simpler use cases where only success/failure is needed.
    pub fn signal(tx: OneshotSenderOf<C, Result<(), io::Error>>) -> Self {
        Self::Signal(tx)
    }

    pub(crate) fn new(io_id: IOId<C>, tx: WatchSenderOf<C, Result<IOId<C>, StorageError<C>>>) -> Self {
        Self::Notify(IOFlushedNotify { io_id, tx })
    }

    /// Report log io completion event (deprecated).
    #[deprecated(since = "0.10.0", note = "Use `io_completed` instead")]
    pub fn log_io_completed(self, result: Result<(), io::Error>) {
        self.io_completed(result)
    }

    /// Report log io completion event.
    ///
    /// It will be called when the log is successfully appended to the storage or an error occurs.
    pub fn io_completed(self, result: Result<(), io::Error>) {
        match self {
            Self::Noop => {}
            Self::Signal(tx) => {
                tx.send(result).ok();
            }
            Self::Notify(IOFlushedNotify { io_id, tx }) => {
                let new_value = match result {
                    Err(e) => {
                        let sto_err = Self::make_storage_error(&io_id, e);
                        Err(sto_err)
                    }
                    Ok(_) => {
                        tracing::debug!("{}: IOFlushed completed: {}", func_name!(), io_id);
                        Ok(io_id)
                    }
                };

                // Use send_if_modified to ensure error permanence:
                // once the value becomes Err, never change back to Ok
                let modified = tx.send_if_modified(move |current| {
                    if current.is_err() {
                        // Already in error state, don't modify
                        tracing::debug!("IO completion ignored: already in error state");
                        false
                    } else {
                        // Update to new value
                        *current = new_value;
                        true
                    }
                });

                if !modified {
                    tracing::debug!("IO completion not sent: channel state unchanged");
                }
            }
        }
    }

    /// Figure out the error subject and verb from the IO ID.
    fn make_storage_error(io_id: &IOId<C>, e: io::Error) -> StorageError<C> {
        tracing::error!("io_completed: IOFlushed error: {}, while flushing IO: {}", e, io_id);

        let subject = io_id.subject();
        let verb = io_id.verb();
        StorageError::from_io_error(subject, verb, e)
    }
}

/// A oneshot callback for completion of applying logs to the state machine.
pub struct LogApplied<C>
where C: RaftTypeConfig
{
    last_log_id: LogIdOf<C>,
    tx: OneshotSenderOf<C, Result<(LogIdOf<C>, Vec<C::R>), StorageError<C>>>,
}

impl<C> LogApplied<C>
where C: RaftTypeConfig
{
    #[allow(dead_code)]
    pub(crate) fn new(
        last_log_id: LogIdOf<C>,
        tx: OneshotSenderOf<C, Result<(LogIdOf<C>, Vec<C::R>), StorageError<C>>>,
    ) -> Self {
        Self { last_log_id, tx }
    }

    /// Report apply io completion event.
    ///
    /// It will be called when the log is successfully applied to the state machine or an error
    /// occurs.
    pub fn completed(self, result: Result<Vec<C::R>, StorageError<C>>) {
        let res = match result {
            Ok(x) => {
                tracing::debug!("LogApplied up to {}", self.last_log_id);
                let resp = (self.last_log_id.clone(), x);
                self.tx.send(Ok(resp))
            }
            Err(e) => {
                tracing::error!("LogApplied error: {}, while applying up to {}", e, self.last_log_id);
                self.tx.send(Err(e))
            }
        };

        if let Err(_e) = res {
            tracing::error!("failed to send apply complete event, last_log_id: {}", self.last_log_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::testing::UTConfig;
    use crate::type_config::TypeConfigExt;

    #[tokio::test]
    async fn test_io_flushed_noop() {
        let callback = IOFlushed::<UTConfig>::noop();
        // Should not panic or do anything
        callback.io_completed(Ok(()));
    }

    #[tokio::test]
    async fn test_io_flushed_noop_with_error() {
        let callback = IOFlushed::<UTConfig>::noop();
        // Should not panic even with error
        callback.io_completed(Err(io::Error::other("test error")));
    }

    #[tokio::test]
    async fn test_io_flushed_signal_success() {
        let (tx, rx) = UTConfig::<()>::oneshot();
        let callback = IOFlushed::<UTConfig>::signal(tx);

        callback.io_completed(Ok(()));

        let result = rx.await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_ok());
    }

    #[tokio::test]
    async fn test_io_flushed_signal_with_error() {
        let (tx, rx) = UTConfig::<()>::oneshot();
        let callback = IOFlushed::<UTConfig>::signal(tx);

        // Signal sends the error result
        callback.io_completed(Err(io::Error::other("test error")));

        let result = rx.await;
        assert!(result.is_ok());
        let io_result = result.unwrap();
        assert!(io_result.is_err());
        assert_eq!(io_result.unwrap_err().kind(), io::ErrorKind::Other);
    }

    #[tokio::test]
    async fn test_io_flushed_signal_receiver_dropped() {
        let (tx, rx) = UTConfig::<()>::oneshot();
        drop(rx);

        let callback = IOFlushed::<UTConfig>::signal(tx);
        // Should not panic when receiver is dropped
        callback.io_completed(Ok(()));
    }
}
