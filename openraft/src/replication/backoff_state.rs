//! Backoff state for replication.
//!
//! Owns the two pieces of state that together decide whether a replication session
//! should throttle outgoing requests after transient RPC errors:
//!
//! - `rank`: accumulated error weight, reset on success.
//! - `inner`: the active [`Backoff`] iterator. Handed to the request-stream generator as a
//!   [`BackoffConsumer`] so the consumer can sample the next delay without being able to enable or
//!   disable the backoff itself.
//!
//! Both pieces must be cleared on success; otherwise a stale [`Backoff`] left over
//! from a prior error throttles every subsequent request in the same stream session
//! until the session ends (see
//! [issue #1723](https://github.com/databendlabs/openraft/issues/1723)).

use std::sync::Arc;
use std::sync::Mutex;

use crate::RaftTypeConfig;
use crate::errors::RPCError;
use crate::network::Backoff;
use crate::replication::backoff_consumer::BackoffConsumer;

/// Error-rank threshold above which backoff is enabled.
const BACKOFF_RANK_THRESHOLD: u64 = 20;

/// Coordinates rank accumulation and the shared [`Backoff`] iterator for a
/// replication session.
///
/// Owned by `ReplicationCore`. The request-stream generator receives a read-only
/// [`BackoffConsumer`] that can only sample delays.
pub(crate) struct BackoffState {
    rank: u64,
    inner: Arc<Mutex<Option<Backoff>>>,
}

impl BackoffState {
    pub(crate) fn new() -> Self {
        Self {
            rank: 0,
            inner: Arc::new(Mutex::new(None)),
        }
    }

    /// Returns a consumer handle that can only sample the current backoff delay.
    pub(crate) fn consumer(&self) -> BackoffConsumer {
        BackoffConsumer {
            inner: self.inner.clone(),
        }
    }

    /// Called when a replication RPC succeeds: clears both `rank` and `inner`.
    ///
    /// Clearing `inner` here is critical — the outer main loop cannot run while a
    /// stream session is active, so this is the only opportunity to drop the stale
    /// [`Backoff`] before the next RPC in the same session reads it.
    ///
    /// Fast path: `rank == 0` implies `inner` is already `None` (invariant maintained
    /// by every writer in this module), so we skip the lock on the common hot path of
    /// successive successes.
    pub(crate) fn on_success(&mut self) {
        if self.rank == 0 {
            return;
        }
        self.rank = 0;
        *self.inner.lock().unwrap() = None;
    }

    /// Called when a replication RPC fails; `weight` is the error's backoff rank.
    pub(crate) fn on_error(&mut self, weight: u64) {
        self.rank += weight;
    }

    /// Dispatches to [`Self::on_success`] or [`Self::on_error`] based on the RPC
    /// outcome, extracting the error weight from [`RPCError::backoff_rank`].
    pub(crate) fn observe<T, C>(&mut self, result: &Result<T, RPCError<C>>)
    where C: RaftTypeConfig {
        match result {
            Ok(_) => self.on_success(),
            Err(e) => self.on_error(e.backoff_rank()),
        }
    }

    /// Reconciles `inner` with `rank` before starting a new stream session:
    /// - If `rank` exceeds the threshold, enables backoff (if not already active).
    /// - Otherwise, clears backoff.
    pub(crate) fn reconcile(&self, backoff_factory: impl FnOnce() -> Backoff) {
        let mut inner = self.inner.lock().unwrap();
        if self.rank > BACKOFF_RANK_THRESHOLD {
            if inner.is_none() {
                *inner = Some(backoff_factory());
            }
        } else {
            *inner = None;
        }
    }

    #[cfg(test)]
    fn is_enabled(&self) -> bool {
        self.inner.lock().unwrap().is_some()
    }

    #[cfg(test)]
    fn rank(&self) -> u64 {
        self.rank
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn make_test_backoff() -> Backoff {
        Backoff::new(std::iter::repeat(Duration::from_millis(200)))
    }

    #[test]
    fn on_error_accumulates_rank_and_reconcile_enables_backoff() {
        let mut state = BackoffState::new();
        assert!(!state.is_enabled());
        assert_eq!(state.rank(), 0);

        // An `Unreachable` error contributes weight 100, which exceeds the threshold.
        state.on_error(100);
        state.reconcile(make_test_backoff);

        assert!(state.is_enabled(), "backoff should be enabled after error > threshold");
        assert_eq!(state.rank(), 100);
    }

    /// Reproduces issue #1723.
    ///
    /// During an active stream session, `on_success` is called on every successful
    /// response, but `reconcile` is only called in the outer main loop and does not
    /// run until the stream session ends. If `on_success` does not itself clear
    /// `inner`, the shared [`Backoff`] remains `Some` indefinitely and every
    /// subsequent request pays the full backoff sleep.
    ///
    /// The invariant this test asserts: **after a successful RPC, the state is
    /// fully reset — both `rank` and `inner` — without relying on an external
    /// reconciliation step.**
    #[test]
    fn on_success_clears_backoff_without_external_reconcile() {
        let mut state = BackoffState::new();

        // 1. Errors accumulate and backoff is enabled (via reconcile at the top of the main loop before a
        //    stream session starts).
        state.on_error(100);
        state.reconcile(make_test_backoff);
        assert!(state.is_enabled(), "precondition: backoff is enabled");

        // 2. A successful response is received inside the stream session — the only signal that reaches the
        //    backoff state during the session is on_success.
        state.on_success();

        assert_eq!(state.rank(), 0, "rank must be reset on success");
        assert!(
            !state.is_enabled(),
            "backoff must be cleared on success, not left for an external reconcile call \
             that does not run during an active stream session"
        );
    }

    /// `observe` dispatches to `on_success` or `on_error` based on the [`Result`]
    /// variant, using [`RPCError::backoff_rank`] to determine the error weight.
    #[test]
    fn observe_dispatches_to_on_success_or_on_error() {
        use crate::engine::testing::UTConfig;
        use crate::errors::Unreachable;

        let mut state = BackoffState::new();

        // Err arm: accumulates rank using the error's `backoff_rank` (100 for Unreachable).
        let err: Result<(), RPCError<UTConfig>> = Err(RPCError::Unreachable(Unreachable::<UTConfig>::from_string("x")));
        state.observe(&err);
        assert_eq!(state.rank(), 100, "err arm: rank accumulated from Unreachable weight");

        // Bring backoff into the enabled state before the Ok arm.
        state.reconcile(make_test_backoff);
        assert!(state.is_enabled(), "precondition: backoff is enabled");

        // Ok arm: clears rank and inner (the fix for issue #1723).
        let ok: Result<(), RPCError<UTConfig>> = Ok(());
        state.observe(&ok);
        assert_eq!(state.rank(), 0, "ok arm: rank reset");
        assert!(!state.is_enabled(), "ok arm: inner cleared");
    }
}
