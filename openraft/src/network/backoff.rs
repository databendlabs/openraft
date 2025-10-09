use std::time::Duration;

use crate::OptionalSend;

/// A backoff instance that is an infinite iterator of durations to sleep before next retry, when a
/// [`Unreachable`](`crate::error::Unreachable`) occurs.
pub struct Backoff {
    #[cfg(not(feature = "singlethreaded"))]
    inner: Box<dyn Iterator<Item = Duration> + Send + 'static>,
    #[cfg(feature = "singlethreaded")]
    inner: Box<dyn Iterator<Item = Duration> + 'static>,
}

impl Backoff {
    /// Create a new Backoff from an iterator of durations.
    pub fn new(iter: impl Iterator<Item = Duration> + OptionalSend + 'static) -> Self {
        Self { inner: Box::new(iter) }
    }
}

impl Iterator for Backoff {
    type Item = Duration;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}
