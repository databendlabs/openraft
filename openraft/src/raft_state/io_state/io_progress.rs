use std::error::Error;
use std::fmt;

use validit::less_equal;
use validit::Validate;

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
/// `(flushed, submitted]` represent the window of I/O operations in progress.
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

impl<T> Default for IOProgress<T>
where T: PartialOrd + fmt::Debug
{
    fn default() -> Self {
        Self {
            accepted: None,
            submitted: None,
            flushed: None,
        }
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
            "flushed/submitted:({}, {}], accepted: {}",
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
    /// Update the `accept` cursor of the I/O progress.
    pub(crate) fn accept(&mut self, new_accepted: T) {
        debug_assert!(
            self.accepted.as_ref().map_or(true, |accepted| accepted <= &new_accepted),
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
            self.submitted.as_ref().map_or(true, |submitted| submitted <= &new_submitted),
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
            self.flushed.as_ref().map_or(true, |flushed| flushed <= &new_flushed),
            "expect flushed:{} < new_flushed:{}",
            self.flushed.display(),
            new_flushed,
        );
        self.flushed = Some(new_flushed);
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
