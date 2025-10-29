//! Callbacks used by Storage API

use std::io;

use crate::ErrorSubject;
use crate::ErrorVerb;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::async_runtime::MpscWeakSender;
use crate::core::notification::Notification;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::MpscWeakSenderOf;
use crate::type_config::alias::OneshotSenderOf;
use crate::type_config::async_runtime::mpsc::MpscSender;
use crate::type_config::async_runtime::oneshot::OneshotSender;

/// Type alias for log flush callback (deprecated).
#[deprecated(since = "0.10.0", note = "Use `IOFlushed` instead")]
pub type LogFlushed<C> = IOFlushed<C>;

/// A callback for completion of io operation to [`RaftLogStorage`].
///
/// [`RaftLogStorage`]: `crate::storage::RaftLogStorage`
pub struct IOFlushed<C>
where C: RaftTypeConfig
{
    /// The notification to send when the IO complete.
    notification: Notification<C>,

    tx: MpscWeakSenderOf<C, Notification<C>>,
}

impl<C> IOFlushed<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(notify: Notification<C>, tx: MpscWeakSenderOf<C, Notification<C>>) -> Self {
        Self {
            notification: notify,
            tx,
        }
    }

    /// Report log io completion event (deprecated).
    #[deprecated(since = "0.10.0", note = "Use `io_completed` instead")]
    pub async fn log_io_completed(self, result: Result<(), io::Error>) {
        self.io_completed(result).await
    }

    /// Report log io completion event.
    ///
    /// It will be called when the log is successfully appended to the storage or an error occurs.
    pub async fn io_completed(self, result: Result<(), io::Error>) {
        let Some(tx) = self.tx.upgrade() else {
            tracing::warn!("failed to upgrade tx, RaftCore may have closed the receiver");
            return;
        };

        let send_res = match result {
            Err(e) => {
                tracing::error!(
                    "{}: IOFlushed error: {}, while flushing IO: {}",
                    func_name!(),
                    e,
                    self.notification
                );

                let sto_err = self.make_storage_error(e);
                tx.send(Notification::StorageError { error: sto_err }).await
            }
            Ok(_) => {
                tracing::debug!("{}: IOFlushed completed: {}", func_name!(), self.notification);
                tx.send(self.notification).await
            }
        };

        if let Err(e) = send_res {
            tracing::warn!("failed to send log io completion event: {}", e.0);
        }
    }

    /// Figure out the error subject and verb from the kind of response `Notification`.
    fn make_storage_error(&self, e: io::Error) -> StorageError<C> {
        match &self.notification {
            Notification::VoteResponse { .. } => StorageError::from_io_error(ErrorSubject::Vote, ErrorVerb::Write, e),
            Notification::LocalIO { io_id } => {
                let subject = io_id.subject();
                let verb = io_id.verb();
                StorageError::from_io_error(subject, verb, e)
            }
            Notification::HigherVote { .. }
            | Notification::StorageError { .. }
            | Notification::ReplicationProgress { .. }
            | Notification::HeartbeatProgress { .. }
            | Notification::StateMachine { .. }
            | Notification::Tick { .. } => {
                unreachable!("Unexpected notification: {}", self.notification)
            }
        }
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
