use std::ops::Deref;
use std::time::Duration;

use openraft_macros::since;

use crate::Raft;
use crate::RaftTypeConfig;
use crate::async_runtime::watch::WatchReceiver;
use crate::errors::Fatal;
use crate::metrics::WaitError;
use crate::raft::linearizable_read::LinearizeState;
use crate::storage::RaftStateMachine;
use crate::type_config::alias::LogIdOf;

/// Represents a linearization operation for read.
///
/// See the [read protocol documentation](crate::docs::protocol::read) for more details.
///
/// This struct is the result returned from [`Raft::get_read_linearizer()`],
/// which is the implementation of awaiting the applied log entries.
/// The application calls [`Linearizer::try_await_ready()`](Self::try_await_ready) to ensure its
/// following reads are linearized.
///
/// It retains the no-op and committed log IDs captured when the read was authorized. Their
/// maximum is the required read log ID; a relaxed read requires only the no-op log ID. It also
/// tracks last known applied progress on a node.
///
/// [`Raft::get_read_linearizer()`]: Raft::get_read_linearizer
#[since(version = "0.10.0")]
#[must_use = "call `try_await_ready()` to ensure linearizability"]
#[derive(Debug, Clone)]
pub struct Linearizer<C>
where C: RaftTypeConfig
{
    /// Captured read boundaries and last known applied progress.
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
    /// Create a linearizer from the captured no-op and committed log IDs and applied progress.
    ///
    /// When transferring a linearizer to a follower, preserve both log IDs from the leader.
    /// `applied` belongs to `node_id` and is not reused as another node's progress.
    #[since(version = "0.10.1", change = "accept separate no-op and committed log IDs")]
    #[since(version = "0.10.0", change = "accept ReadLogId instead of LogId")]
    #[since(version = "0.10.0")]
    pub fn new(
        node_id: C::NodeId,
        noop_log_id: LogIdOf<C>,
        committed: Option<LogIdOf<C>>,
        applied: Option<LogIdOf<C>>,
    ) -> Self {
        Self {
            state: LinearizeState::new(node_id, noop_log_id, committed, applied),
        }
    }

    /// Mark whether this read requires only the no-op log ID of the authorizing leadership.
    ///
    /// The captured committed log ID is retained either way. Only `Raft::read_relaxed` requests
    /// relaxed reads, because it validates the sampled snapshot before exposing a result.
    pub(crate) fn with_relaxed(self, relaxed: bool) -> Self {
        let state = self.state.with_relaxed(relaxed);
        Self { state }
    }

    /// Replace the last applied log id this linearizer reports.
    ///
    /// A queued read observes the applied log id when it is created, which may lag by a heartbeat
    /// round when the read is finally answered. Refreshing it lets the caller skip waiting for
    /// apply progress the leader has already made.
    pub(crate) fn with_applied(self, applied: Option<LogIdOf<C>>) -> Self {
        let node_id = self.state.node_id().clone();
        let state = self.state.with_applied(node_id, applied);
        Self { state }
    }

    /// Waits indefinitely for the state machine to apply all required log entries for linearizable
    /// reads.
    ///
    /// This is a convenience method that calls [`try_await_ready(_, None)`](Self::try_await_ready)
    /// with no timeout and unwraps the result.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let linearizer = raft.get_read_linearizer().await?;
    /// let state = linearizer.await_ready(&raft).await?;
    /// // Now safe to perform linearizable reads
    /// ```
    #[since(version = "0.10.0")]
    pub async fn await_ready<SM>(self, raft: &Raft<C, SM>) -> Result<LinearizeState<C>, Fatal<C>>
    where SM: RaftStateMachine<C> {
        let state_res = self.try_await_ready(raft, None).await?;
        // Safe unwrap: No timeout error.
        Ok(state_res.unwrap())
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
    /// let state = linearizer.try_await_ready(&raft, Some(Duration::from_secs(1))).await??;
    /// // Now safe to perform linearizable reads
    /// ```
    #[since(version = "0.10.0")]
    pub async fn try_await_ready<SM>(
        self,
        raft: &Raft<C, SM>,
        timeout: Option<Duration>,
    ) -> Result<Result<LinearizeState<C>, LinearizeState<C>>, Fatal<C>>
    where
        SM: RaftStateMachine<C>,
    {
        let this_id = raft.inner.id();

        if self.state.is_ready_on_node(this_id) {
            return Ok(Ok(self.state));
        }

        let expected = Some(self.state.read_log_id().index());

        let res = raft.inner.wait(timeout).applied_index_at_least(expected, "Linearizer::try_await_ready").await;

        match res {
            Ok(metrics) => Ok(Ok(self.state.with_applied(this_id.clone(), metrics.last_applied))),
            Err(e) => match e {
                WaitError::Timeout(_, _) => {
                    let metrics_rx = raft.metrics();
                    let ref_metrics = metrics_rx.borrow_watched();
                    let applied = ref_metrics.last_applied.clone();

                    let state = self.state.with_applied(this_id.clone(), applied);
                    if state.is_ready_on_node(this_id) {
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
