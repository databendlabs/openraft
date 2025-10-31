use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftTypeConfig;
use crate::Vote;
use crate::async_runtime::watch::RecvError;
use crate::async_runtime::watch::WatchReceiver;
use crate::core::io_flush_tracking::FlushPoint;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::WatchReceiverOf;

/// Handle for tracking log I/O flush progress.
///
/// Returns `None` if no I/O has completed yet (e.g., on a newly started node before any writes).
/// Returns `Some(FlushPoint)` containing the vote and last log ID after the first flush
/// completes.
pub type LogProgress<C> = WatchProgress<C, Option<FlushPoint<C>>>;

/// Handle for tracking vote I/O flush progress.
///
/// Returns `None` if no vote has been flushed yet.
/// Returns `Some(Vote)` containing the last flushed vote.
pub type VoteProgress<C> = WatchProgress<C, Option<Vote<C>>>;

/// Handle for tracking commit log progress.
///
/// Returns `None` if no log has been committed yet.
/// Returns `Some(LogId)` containing the latest committed log id.
pub type CommitProgress<C> = WatchProgress<C, Option<LogIdOf<C>>>;

/// Handle for tracking snapshot persistence progress.
///
/// Returns `None` if no snapshot has been persisted yet.
/// Returns `Some(LogId)` containing the last persisted snapshot log id.
pub type SnapshotProgress<C> = WatchProgress<C, Option<LogIdOf<C>>>;

/// Handle for tracking applied log progress.
///
/// Returns `None` if no log has been applied yet.
/// Returns `Some(LogId)` containing the last applied log id.
pub type AppliedProgress<C> = WatchProgress<C, Option<LogIdOf<C>>>;

/// Watch handle for tracking I/O flush progress.
///
/// Provides three operations:
/// - [`get()`](Self::get): Get current progress state immediately
/// - [`wait_until_ge()`](Self::wait_until_ge): Wait asynchronously until progress reaches a
///   threshold
/// - [`wait_until()`](Self::wait_until): Wait asynchronously until progress satisfies a custom
///   condition
///
/// # Concurrency
///
/// - Multiple handles can watch concurrently (each clones the receiver)
/// - `get()` provides a snapshot at call time (may be stale immediately)
/// - `wait_until_ge()` is sequentially consistent: if it returns, all future `get()` calls will see
///   a value >= the returned value
///
/// This is a thin wrapper around a watch channel receiver that enforces the progress
/// tracking semantics (values must be comparable via `PartialOrd`).
#[derive(Clone)]
pub struct WatchProgress<C, T>
where
    C: RaftTypeConfig,
    T: OptionalSend + OptionalSync + PartialOrd + Clone,
{
    inner: WatchReceiverOf<C, T>,
}

impl<C, T> WatchProgress<C, T>
where
    C: RaftTypeConfig,
    T: OptionalSend + OptionalSync + PartialOrd + Clone,
{
    pub(crate) fn new(inner: WatchReceiverOf<C, T>) -> Self {
        Self { inner }
    }

    /// Wait until the flushed I/O progress becomes greater than or equal to the target value.
    ///
    /// Returns the current progress state once the condition is satisfied. If the progress
    /// is already >= `target`, returns immediately.
    ///
    /// # Errors
    ///
    /// Returns `RecvError` if the sender is dropped (node is shutting down).
    ///
    /// # Example
    ///
    /// ```ignore
    /// let target = Some(FlushPoint::new(Vote::new(2, node_id), Some(log_id(2, node_id, 100))));
    /// let state = log_progress.wait_until_ge(&target).await?;
    /// // state is guaranteed to be >= target
    /// ```
    pub async fn wait_until_ge(&mut self, target: &T) -> Result<T, RecvError> {
        self.inner.wait_until_ge(target).await
    }

    /// Wait until the flushed I/O progress satisfies the given condition.
    ///
    /// Returns the current progress state once the condition is satisfied. If the progress
    /// already satisfies the condition, returns immediately.
    ///
    /// # Errors
    ///
    /// Returns `RecvError` if the sender is dropped (node is shutting down).
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Wait until vote term is exactly 5
    /// let state = vote_progress.wait_until(|v| v.as_ref().map_or(false, |vote| vote.leader_id().term == 5)).await?;
    /// ```
    pub async fn wait_until<F>(&mut self, condition: F) -> Result<T, RecvError>
    where F: Fn(&T) -> bool + OptionalSend {
        self.inner.wait_until(condition).await
    }

    /// Get the current flushed I/O progress state immediately without waiting.
    ///
    /// This returns a snapshot of the most recent flushed I/O operation. The value may become
    /// stale immediately after reading as new I/O operations complete concurrently.
    pub fn get(&self) -> T {
        self.inner.borrow_watched().clone()
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::RaftTypeConfig;
    use crate::impls::TokioRuntime;
    use crate::impls::Vote;
    use crate::type_config::TypeConfigExt;

    #[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd)]
    struct TestConfig;

    impl RaftTypeConfig for TestConfig {
        type D = u64;
        type R = ();
        type NodeId = u64;
        type Node = ();
        type Term = u64;
        type LeaderId = crate::impls::leader_id_adv::LeaderId<Self>;
        type Vote = Vote<Self>;
        type Entry = crate::impls::Entry<Self>;
        type SnapshotData = Cursor<Vec<u8>>;
        type AsyncRuntime = TokioRuntime;
        type Responder<T>
            = crate::impls::OneshotResponder<Self, T>
        where T: OptionalSend + 'static;
    }

    #[tokio::test]
    async fn test_wait_until_ge() {
        let (tx, rx) = TestConfig::watch_channel(0u64);
        let mut progress = WatchProgress::<TestConfig, u64>::new(rx);

        assert_eq!(progress.get(), 0);

        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            tx.send(5).unwrap();
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            tx.send(10).unwrap();
        });

        let result = progress.wait_until_ge(&8).await.unwrap();
        assert!(result >= 8);
        assert_eq!(result, 10);
    }

    #[tokio::test]
    async fn test_wait_until_ge_immediate() {
        let (tx, rx) = TestConfig::watch_channel(10u64);
        let mut progress = WatchProgress::<TestConfig, u64>::new(rx);

        let result = progress.wait_until_ge(&5).await.unwrap();
        assert_eq!(result, 10);

        drop(tx);
    }

    #[tokio::test]
    async fn test_wait_until_custom_condition() {
        let (tx, rx) = TestConfig::watch_channel(1u64);
        let mut progress = WatchProgress::<TestConfig, u64>::new(rx);

        tokio::spawn(async move {
            for i in 2..=10 {
                tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
                tx.send(i).unwrap();
            }
        });

        let result = progress.wait_until(|v| v % 2 == 0).await.unwrap();
        assert_eq!(result % 2, 0);
        assert_eq!(result, 2);
    }

    #[tokio::test]
    async fn test_wait_until_immediate() {
        let (tx, rx) = TestConfig::watch_channel(10u64);
        let mut progress = WatchProgress::<TestConfig, u64>::new(rx);

        let result = progress.wait_until(|v| v >= &5).await.unwrap();
        assert_eq!(result, 10);

        drop(tx);
    }
}
