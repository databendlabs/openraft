use std::time::Duration;

/// A backoff instance that is an infinite iterator of durations to sleep before next retry, when a
/// [`Unreachable`](`crate::error::Unreachable`) occurs.
pub struct Backoff {
    inner: Box<dyn Iterator<Item = Duration> + Send + 'static>,
}

impl Backoff {
    pub fn new(iter: impl Iterator<Item = Duration> + Send + 'static) -> Self {
        Self { inner: Box::new(iter) }
    }
}

impl Iterator for Backoff {
    type Item = Duration;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}
