use std::ops::Deref;
use std::time::Duration;

use openraft_macros::since;

use crate::async_runtime::watch::WatchReceiver;
use crate::error::Fatal;
use crate::metrics::WaitError;
use crate::raft::linearizable_read::LinearizeState;
use crate::LogId;
use crate::Raft;
use crate::RaftTypeConfig;

/// Linearizer represents a linearization operation for read.
///
/// See the [read protocol documentation](crate::docs::protocol::read) for more details.
///
/// This struct is the result returned from [`Raft::get_read_linearizer()`],
/// which is the implementation of awaiting the applied log entries.
/// The application calls [`Linearizer::await_applied()`](Self::await_applied) to ensure its
/// following reads are linearized.
///
/// It contains:
/// - a `read_log_id`: the log ID that must be applied before reading to ensure linearizability
/// - a `applied`: the last known log ID that has been applied to the state machine
///
/// [`Raft::get_read_linearizer()`]: Raft::get_read_linearizer
#[since(version = "0.10.0")]
#[must_use = "call `await_applied()` to ensure linearizability"]
#[derive(Debug, Clone)]
pub struct Linearizer<C>
where C: RaftTypeConfig
{
    /// The state containing the read log ID and last applied log ID for linearizable reads.
    state: LinearizeState<C>,
}

impl<C> Deref for Linearizer<C>
where C: RaftTypeConfig
{
    type Target = LinearizeState<C>;

    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl<C> Linearizer<C>
where C: RaftTypeConfig
{
    #[since(version = "0.10.0")]
    pub fn new(read_log_id: LogId<C>, applied: Option<LogId<C>>) -> Self {
        Self {
            state: LinearizeState::new(read_log_id, applied),
        }
    }

    /// Waits for the state machine to apply all required log entries for linearizable reads.
    ///
    /// This method ensures linearizability by waiting for the state machine to apply all log
    /// entries up to the `read_log_id`.
    ///
    /// # Returns
    ///
    /// Returns `Ok(Ok(LinearizeState))` once `applied >= read_log_id`, indicating it's safe to
    /// perform linearizable reads.
    ///
    /// If `timeout` is provided and expires, returns `Ok(Err(LinearizeState))` where
    /// `applied < read_log_id`, indicating the read cannot be performed yet.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let linearizer = raft.get_read_linearizer().await?;
    /// let state = linearizer.await_applied(&raft, Some(Duration::from_secs(1))).await??;
    /// // Now safe to perform linearizable reads
    /// ```
    #[since(version = "0.10.0")]
    pub async fn await_applied(
        self,
        raft: &Raft<C>,
        timeout: Option<Duration>,
    ) -> Result<Result<LinearizeState<C>, LinearizeState<C>>, Fatal<C>> {
        // TODO: test timeout
        if self.state.is_ready() {
            return Ok(Ok(self.state));
        }

        let expected = Some(self.state.read_log_id().index());

        let res = raft.inner.wait(timeout).applied_index_at_least(expected, "Linearizer::await_applied").await;

        match res {
            Ok(metrics) => Ok(Ok(self.state.with_applied(metrics.last_applied))),
            Err(e) => match e {
                WaitError::Timeout(_, _) => {
                    let metrics_rx = raft.metrics();
                    let ref_metrics = metrics_rx.borrow_watched();
                    let applied = ref_metrics.last_applied.clone();

                    let state = self.state.with_applied(applied);
                    if state.is_ready() {
                        Ok(Ok(state))
                    } else {
                        Ok(Err(state))
                    }
                }
                WaitError::ShuttingDown => {
                    let err = raft.inner.get_core_stop_error().await;
                    Err(err)
                }
            },
        }
    }
}
