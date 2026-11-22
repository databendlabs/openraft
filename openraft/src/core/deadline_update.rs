/// What [`DeadlineNotifier::set_deadline()`](crate::core::DeadlineNotifier::set_deadline) did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DeadlineUpdate {
    /// The stored deadline changed.
    pub(crate) changed: bool,

    /// The worker was woken to read the new deadline and re-arm its timer.
    ///
    /// The worker is woken only when it could otherwise sleep past the new deadline: when the new
    /// deadline is earlier than the stored one, or the stored one is `None` or has passed.
    pub(crate) woke: bool,
}
