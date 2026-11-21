use std::time::Duration;

use crate::OptionalSend;
use crate::base::BoxIterator;

/// The delays between the retries of an RPC to a node that is unreachable.
///
/// After an [`Unreachable`](`crate::error::Unreachable`) error, Openraft sleeps for the next
/// delay of this iterator before it retries. Supply an infinite iterator, such as one that ends
/// by repeating its last delay: the end of the iterator does not stop the retries. Whenever it
/// returns `None`, Openraft retries after a fixed delay of its own choosing, an implementation
/// default that may change.
///
/// Every policy of [`Config::backoff`](crate::Config::backoff) is infinite.
pub struct Backoff {
    inner: BoxIterator<'static, Duration>,
}

impl Backoff {
    /// Create a new Backoff from an iterator of durations.
    ///
    /// `iter` should be infinite: see [`Backoff`] for what happens after it ends.
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
