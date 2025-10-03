use std::error::Error;
use std::fmt;

use validit::Validate;
use validit::less_equal;

use crate::display_ext::DisplayOptionExt;

/// Tracks the progress of some non-reorder-able I/O operations.
///
/// It keeps track of two key stages in the I/O process: submission to the queue and flushing to
/// storage.
///
/// `T`: A totally ordered type representing the I/O operation identifier. This could be a
/// sequence number, timestamp, or any other comparable value.
///
/// `accepted`: The id of the last IO operation accepted, not yet submitted.
/// `submitted`: The id of the last IO operation submitted to the queue.
/// `flushed`: The id of the last IO operation successfully flushed to storage.
///
/// `(flushed, submitted]` represents the window of I/O operations in progress.
///
/// Invariants:
/// ```text
/// flushed <= submitted <= accepted
/// ```
#[derive(Debug, Clone)]
#[derive(PartialEq, Eq)]
pub(crate) struct IOProgress<T>
where T: PartialOrd + fmt::Debug
{
    accepted: Option<T>,
    submitted: Option<T>,
    flushed: Option<T>,
    name: &'static str,
}

impl<T> Validate for IOProgress<T>
where T: PartialOrd + fmt::Debug
{
    fn validate(&self) -> Result<(), Box<dyn Error>> {
        less_equal!(&self.flushed, &self.submitted);
        less_equal!(&self.submitted, &self.accepted);
        Ok(())
    }
}

impl<T> fmt::Display for IOProgress<T>
where
    T: PartialOrd + fmt::Debug,
    T: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:(flushed/submitted:({}, {}], accepted: {})",
            self.name,
            self.flushed.display(),
            self.submitted.display(),
            self.accepted.display(),
        )
    }
}

impl<T> IOProgress<T>
where
    T: PartialOrd + fmt::Debug,
    T: fmt::Display,
{
    /// Create a new IOProgress with all three cursors (accepted, submitted, flushed) set to the
    /// same value.
    ///
    /// This creates a synchronized state where all IO operations (accepted, submitted, and flushed)
    /// are considered complete up to the specified point. This is typically used for initialization
    /// or when a snapshot is installed, ensuring all IO tracking is aligned.
    pub(crate) fn new_synchronized(v: Option<T>, name: &'static str) -> Self
    where T: Clone {
        Self {
            accepted: v.clone(),
            submitted: v.clone(),
            flushed: v.clone(),
            name,
        }
    }

    /// Update the `accept` cursor of the I/O progress.
    pub(crate) fn accept(&mut self, new_accepted: T) {
        debug_assert!(
            self.accepted.as_ref().is_none_or(|accepted| accepted <= &new_accepted),
            "expect accepted:{} < new_accepted:{}",
            self.accepted.display(),
            new_accepted,
        );
        self.accepted = Some(new_accepted);
        tracing::debug!("RAFT_io_progress: {}", self);
    }

    /// Update the `submit` cursor of the I/O progress.
    pub(crate) fn submit(&mut self, new_submitted: T) {
        debug_assert!(
            self.submitted.as_ref().is_none_or(|submitted| submitted <= &new_submitted),
            "expect submitted:{} < new_submitted:{}",
            self.submitted.display(),
            new_submitted,
        );
        self.submitted = Some(new_submitted);
        tracing::debug!("RAFT_io_progress: {}", self);
    }

    /// Update the `flush` cursor of the I/O progress.
    pub(crate) fn flush(&mut self, new_flushed: T) {
        debug_assert!(
            self.flushed.as_ref().is_none_or(|flushed| flushed <= &new_flushed),
            "expect flushed:{} < new_flushed:{}",
            self.flushed.display(),
            new_flushed,
        );
        self.flushed = Some(new_flushed);
        tracing::debug!("RAFT_io_progress: {}", self);
    }

    /// Conditionally update all three cursors (accepted, submitted, flushed) to the same value.
    ///
    /// Each cursor is only updated if it is behind the given value, ensuring monotonic progress.
    /// This is primarily used when snapshot building completes asynchronously - the snapshot
    /// progress may have already advanced through normal operations while the snapshot was being
    /// built, so we only update cursors that are actually behind.
    pub(crate) fn try_update_all(&mut self, value: T)
    where T: Clone {
        if self.accepted.as_ref() < Some(&value) {
            self.accept(value.clone());
        }
        if self.submitted.as_ref() < Some(&value) {
            self.submit(value.clone());
        }
        if self.flushed.as_ref() < Some(&value) {
            self.flush(value.clone());
        }

        tracing::debug!("RAFT_io_progress: {}", self);
    }

    pub(crate) fn accepted(&self) -> Option<&T> {
        self.accepted.as_ref()
    }

    // Not used until Command reorder is implemented.
    #[allow(dead_code)]
    pub(crate) fn submitted(&self) -> Option<&T> {
        self.submitted.as_ref()
    }

    pub(crate) fn flushed(&self) -> Option<&T> {
        self.flushed.as_ref()
    }
}
