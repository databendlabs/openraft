//! Callbacks used by Storage API

use std::io;

use tokio::sync::oneshot;

use crate::async_runtime::MpscUnboundedSender;
use crate::async_runtime::MpscUnboundedWeakSender;
use crate::core::notify::Notify;
use crate::type_config::alias::MpscUnboundedWeakSenderOf;
use crate::ErrorSubject;
use crate::ErrorVerb;
use crate::LogId;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::StorageIOError;

#[deprecated(since = "0.10.0", note = "Use `IOFlushed` instead")]
pub type LogFlushed<C> = IOFlushed<C>;

/// A callback for completion of io operation to [`RaftLogStorage`].
///
/// [`RaftLogStorage`]: `crate::storage::RaftLogStorage`
pub struct IOFlushed<C>
where C: RaftTypeConfig
{
    /// The notify to send when the IO complete.
    notify: Notify<C>,

    tx: MpscUnboundedWeakSenderOf<C, Notify<C>>,
}

impl<C> IOFlushed<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(notify: Notify<C>, tx: MpscUnboundedWeakSenderOf<C, Notify<C>>) -> Self {
        Self { notify, tx }
    }

    #[deprecated(since = "0.10.0", note = "Use `io_completed` instead")]
    pub fn log_io_completed(self, result: Result<(), io::Error>) {
        self.io_completed(result)
    }

    /// Report log io completion event.
    ///
    /// It will be called when the log is successfully appended to the storage or an error occurs.
    pub fn io_completed(self, result: Result<(), io::Error>) {
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
                    self.notify
                );

                let sto_err = self.make_storage_error(e);
                tx.send(Notify::StorageError { error: sto_err })
            }
            Ok(_) => {
                tracing::debug!("{}: IOFlushed completed: {}", func_name!(), self.notify);
                tx.send(self.notify)
            }
        };

        if let Err(e) = send_res {
            tracing::error!("failed to send log io completion event: {}", e.0);
        }
    }

    /// Figure out the error subject and verb from the kind of response `Notify`.
    fn make_storage_error(&self, e: io::Error) -> StorageError<C> {
        match &self.notify {
            Notify::VoteResponse { .. } => StorageError::from_io_error(ErrorSubject::Vote, ErrorVerb::Write, e),
            Notify::HigherVote { .. } => {
                unreachable!("")
            }
            Notify::StorageError { .. } => {
                unreachable!("")
            }
            Notify::LocalIO { io_id } => {
                let subject = io_id.subject();
                let verb = io_id.verb();
                StorageError::from_io_error(subject, verb, e)
            }
            Notify::Network { .. } => {
                unreachable!("")
            }
            Notify::StateMachine { .. } => {
                unreachable!("")
            }
            Notify::Tick { .. } => {
                unreachable!("")
            }
        }
    }
}

/// A oneshot callback for completion of applying logs to state machine.
pub struct LogApplied<C>
where C: RaftTypeConfig
{
    last_log_id: LogId<C::NodeId>,
    tx: oneshot::Sender<Result<(LogId<C::NodeId>, Vec<C::R>), StorageIOError<C>>>,
}

impl<C> LogApplied<C>
where C: RaftTypeConfig
{
    #[allow(dead_code)]
    pub(crate) fn new(
        last_log_id: LogId<C::NodeId>,
        tx: oneshot::Sender<Result<(LogId<C::NodeId>, Vec<C::R>), StorageIOError<C>>>,
    ) -> Self {
        Self { last_log_id, tx }
    }

    /// Report apply io completion event.
    ///
    /// It will be called when the log is successfully applied to the state machine or an error
    /// occurs.
    pub fn completed(self, result: Result<Vec<C::R>, StorageIOError<C>>) {
        let res = match result {
            Ok(x) => {
                tracing::debug!("LogApplied upto {}", self.last_log_id);
                let resp = (self.last_log_id, x);
                self.tx.send(Ok(resp))
            }
            Err(e) => {
                tracing::error!("LogApplied error: {}, while applying upto {}", e, self.last_log_id);
                self.tx.send(Err(e))
            }
        };

        if let Err(_e) = res {
            tracing::error!("failed to send apply complete event, last_log_id: {}", self.last_log_id);
        }
    }
}
