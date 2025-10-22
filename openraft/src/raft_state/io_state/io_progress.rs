use std::error::Error;
use std::fmt;

use validit::Validate;
use validit::less_equal;

use crate::display_ext::DisplayOptionExt;

/// Tracks the progress of I/O operations through three stages: accepted, submitted, and flushed.
///
/// `T`: A totally ordered type representing the I/O operation identifier (e.g., [`LogIOId`]).
///
/// Invariant: `flushed <= submitted <= accepted`
///
/// For a comprehensive explanation of the three-stage tracking and examples, see:
/// [Log I/O Progress](crate::docs::data::log_io_progress).
///
/// [`LogIOId`]: crate::raft_state::io_state::log_io_id::LogIOId
#[derive(Debug, Clone)]
#[derive(PartialEq, Eq)]
pub(crate) struct IOProgress<T>
where T: PartialOrd + fmt::Debug
{
    accepted: Option<T>,
    submitted: Option<T>,
    flushed: Option<T>,

    id: String,
    name: &'static str,

    /// Allow IO completion notifications to arrive out of order.
    ///
    /// When enabled, storage may report completions non-monotonically.
    /// For example, `[5..6)` may notify before `[6..8)`.
    ///
    /// Disable to detect bugs if storage guarantees strictly ordered notifications.
    allow_notification_reorder: bool,
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
            "id={:<2} {:>7}:(flushed/submitted:({}, {}], accepted: {})",
            self.id,
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
    /// Create a new IOProgress with all cursors synchronized to the same value.
    ///
    /// Used for initialization or snapshot installation to align all IO tracking.
    ///
    /// - `allow_notification_reorder`: Whether to allow IO completion notifications to arrive out
    ///   of order.
    pub(crate) fn new_synchronized(
        v: Option<T>,
        id: impl ToString,
        name: &'static str,
        allow_notification_reorder: bool,
    ) -> Self
    where
        T: Clone,
    {
        Self {
            accepted: v.clone(),
            submitted: v.clone(),
            flushed: v.clone(),
            id: id.to_string(),
            name,
            allow_notification_reorder,
        }
    }

    /// Update the `accept` cursor of the I/O progress.
    pub(crate) fn accept(&mut self, new_accepted: T) {
        tracing::debug!("RAFT_io    {}; new_accepted: {}", self, new_accepted);

        #[allow(clippy::collapsible_if)]
        if cfg!(debug_assertions) {
            if !self.allow_notification_reorder {
                assert!(
                    self.accepted.as_ref().is_none_or(|accepted| accepted <= &new_accepted),
                    "expect accepted:{} < new_accepted:{}",
                    self.accepted.display(),
                    new_accepted,
                );
            }
        }

        self.accepted = Some(new_accepted);

        tracing::debug!("RAFT_io    {}", self);
    }

    /// Update the `submit` cursor of the I/O progress.
    pub(crate) fn submit(&mut self, new_submitted: T) {
        tracing::debug!("RAFT_io    {}; new_submitted: {}", self, new_submitted);

        #[allow(clippy::collapsible_if)]
        if cfg!(debug_assertions) {
            if !self.allow_notification_reorder {
                assert!(
                    self.submitted.as_ref().is_none_or(|submitted| submitted <= &new_submitted),
                    "expect submitted:{} < new_submitted:{}",
                    self.submitted.display(),
                    new_submitted,
                );
            }
        }

        self.submitted = Some(new_submitted);

        tracing::debug!("RAFT_io    {}", self);
    }

    /// Update the `flush` cursor of the I/O progress.
    pub(crate) fn flush(&mut self, new_flushed: T) {
        tracing::debug!("RAFT_io    {}; new_flushed: {}", self, new_flushed);

        #[allow(clippy::collapsible_if)]
        if cfg!(debug_assertions) {
            if !self.allow_notification_reorder {
                assert!(
                    self.flushed.as_ref().is_none_or(|flushed| flushed <= &new_flushed),
                    "expect flushed:{} < new_flushed:{}",
                    self.flushed.display(),
                    new_flushed,
                );
            }
        }

        self.flushed = Some(new_flushed);

        tracing::debug!("RAFT_io    {}", self);
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

        tracing::debug!("RAFT_io    {}", self);
    }

    /// Conditionally update the `accept` cursor if the new value is greater.
    #[allow(dead_code)]
    pub(crate) fn try_accept(&mut self, new_accepted: T) {
        if self.accepted.as_ref() < Some(&new_accepted) {
            self.accept(new_accepted);
            tracing::debug!("RAFT_io    {}", self);
        }
    }

    /// Conditionally update the `submit` cursor if the new value is greater.
    #[allow(dead_code)]
    pub(crate) fn try_submit(&mut self, new_submitted: T) {
        if self.submitted.as_ref() < Some(&new_submitted) {
            self.submit(new_submitted);
            tracing::debug!("RAFT_io    {}", self);
        }
    }

    /// Conditionally update the `flush` cursor if the new value is greater.
    #[allow(dead_code)]
    pub(crate) fn try_flush(&mut self, new_flushed: T) {
        if self.flushed.as_ref() < Some(&new_flushed) {
            self.flush(new_flushed);
            tracing::debug!("RAFT_io    {}", self);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allow_notification_reorder_disabled() {
        let mut progress = IOProgress::new_synchronized(Some(10), 1, "test", false);

        // Monotonic updates should work
        progress.accept(11);
        assert_eq!(Some(&11), progress.accepted());

        progress.submit(11);
        assert_eq!(Some(&11), progress.submitted());

        progress.flush(11);
        assert_eq!(Some(&11), progress.flushed());
    }

    #[test]
    #[should_panic(expected = "expect accepted:11 < new_accepted:10")]
    #[cfg(debug_assertions)]
    fn test_allow_notification_reorder_disabled_accept_panics() {
        let mut progress = IOProgress::new_synchronized(Some(10), 1, "test", false);
        progress.accept(11);
        progress.accept(10); // Should panic - out of order
    }

    #[test]
    #[should_panic(expected = "expect submitted:11 < new_submitted:10")]
    #[cfg(debug_assertions)]
    fn test_allow_notification_reorder_disabled_submit_panics() {
        let mut progress = IOProgress::new_synchronized(Some(10), 1, "test", false);
        progress.submit(11);
        progress.submit(10); // Should panic - out of order
    }

    #[test]
    #[should_panic(expected = "expect flushed:11 < new_flushed:10")]
    #[cfg(debug_assertions)]
    fn test_allow_notification_reorder_disabled_flush_panics() {
        let mut progress = IOProgress::new_synchronized(Some(10), 1, "test", false);
        progress.flush(11);
        progress.flush(10); // Should panic - out of order
    }

    #[test]
    fn test_allow_notification_reorder_enabled() {
        let mut progress = IOProgress::new_synchronized(Some(10), 1, "test", true);

        // Monotonic updates work
        progress.accept(11);
        assert_eq!(Some(&11), progress.accepted());

        // Out-of-order updates also work
        progress.accept(9);
        assert_eq!(Some(&9), progress.accepted());

        // Test submit
        progress.submit(11);
        assert_eq!(Some(&11), progress.submitted());

        progress.submit(8);
        assert_eq!(Some(&8), progress.submitted());

        // Test flush
        progress.flush(11);
        assert_eq!(Some(&11), progress.flushed());

        progress.flush(7);
        assert_eq!(Some(&7), progress.flushed());
    }

    #[test]
    fn test_try_update_all_respects_reorder_flag() {
        // With reorder disabled
        let mut progress = IOProgress::new_synchronized(Some(10), 1, "test", false);
        progress.try_update_all(15);
        assert_eq!(Some(&15), progress.accepted());
        assert_eq!(Some(&15), progress.submitted());
        assert_eq!(Some(&15), progress.flushed());

        // With reorder enabled
        let mut progress = IOProgress::new_synchronized(Some(10), 1, "test", true);
        progress.try_update_all(15);
        assert_eq!(Some(&15), progress.accepted());
        assert_eq!(Some(&15), progress.submitted());
        assert_eq!(Some(&15), progress.flushed());

        // Try update with smaller value - should not update
        progress.try_update_all(12);
        assert_eq!(Some(&15), progress.accepted());
        assert_eq!(Some(&15), progress.submitted());
        assert_eq!(Some(&15), progress.flushed());
    }

    #[test]
    fn test_try_accept() {
        let mut progress = IOProgress::new_synchronized(Some(10), 1, "test", false);

        // Update with greater value - should update
        progress.try_accept(15);
        assert_eq!(Some(&15), progress.accepted());

        // Update with smaller value - should not update
        progress.try_accept(12);
        assert_eq!(Some(&15), progress.accepted());

        // Update with equal value - should not update
        progress.try_accept(15);
        assert_eq!(Some(&15), progress.accepted());

        // Update with greater value again - should update
        progress.try_accept(20);
        assert_eq!(Some(&20), progress.accepted());
    }

    #[test]
    fn test_try_submit() {
        let mut progress = IOProgress::new_synchronized(Some(10), 1, "test", false);

        // Update with greater value - should update
        progress.try_submit(15);
        assert_eq!(Some(&15), progress.submitted());

        // Update with smaller value - should not update
        progress.try_submit(12);
        assert_eq!(Some(&15), progress.submitted());

        // Update with equal value - should not update
        progress.try_submit(15);
        assert_eq!(Some(&15), progress.submitted());

        // Update with greater value again - should update
        progress.try_submit(20);
        assert_eq!(Some(&20), progress.submitted());
    }

    #[test]
    fn test_try_flush() {
        let mut progress = IOProgress::new_synchronized(Some(10), 1, "test", false);

        // Update with greater value - should update
        progress.try_flush(15);
        assert_eq!(Some(&15), progress.flushed());

        // Update with smaller value - should not update
        progress.try_flush(12);
        assert_eq!(Some(&15), progress.flushed());

        // Update with equal value - should not update
        progress.try_flush(15);
        assert_eq!(Some(&15), progress.flushed());

        // Update with greater value again - should update
        progress.try_flush(20);
        assert_eq!(Some(&20), progress.flushed());
    }
}
