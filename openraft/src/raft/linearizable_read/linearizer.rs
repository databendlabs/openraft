use std::ops::Deref;
use std::time::Duration;

use crate::async_runtime::watch::WatchReceiver;
use crate::error::Fatal;
use crate::metrics::WaitError;
use crate::raft::linearizable_read::LinearizeState;
use crate::LogId;
use crate::Raft;
use crate::RaftTypeConfig;

/// Represents a linearizable read operation.
///
/// This is the result returned from `Raft::get_read_log_id()`, which contains:
/// - a read-log-id: the log ID that must be applied before reading to ensure linearizability
/// - the last applied log ID: the last known log ID that has been applied to the state machine
///
/// This struct is a wrapper for the implementation of Raft Lease Read, which allows linearizable
/// reads without requiring consensus. See the [read protocol
/// documentation](crate::docs::protocol::read) for more details.
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
    pub fn new(read_log_id: LogId<C>, applied: Option<LogId<C>>) -> Self {
        Self {
            state: LinearizeState::new(read_log_id, applied),
        }
    }

    /// Waits on the given `Raft` instance for the state machine to apply all required log entries
    /// for linearizable reads.
    ///
    /// This method ensures linearizability by waiting for the state machine to apply all log
    /// entries up to the `read_log_id` before proceeding with the read operation. If the state
    /// machine has already applied the required entries, this method returns immediately.
    ///
    /// # Returns
    ///
    /// Returns an [`LinearizeState`] that satisfies the linearizable read condition on success.
    /// The returned instance updated its last **applied** log id such that `applied >=
    /// read_log_id`, indicating that it's now safe to perform linearizable reads from the state
    /// machine.
    ///
    /// If `timeout` is provided and the wait operation times out, it returns
    /// `Ok(Err(LinearizedState))`, containing the last applied log ID at the time of the timeout.
    /// In this case, `applied < read_log_id`, indicating that the read operation cannot be
    /// performed.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let linearizer = raft.get_read_linearizer().await?;
    /// let linearized_state = linearizer.await_applied(&raft, Some(Duration::from_secs(1))).await??;
    ///
    /// // Now safe to perform linearizable reads from the state machine
    /// ```
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
