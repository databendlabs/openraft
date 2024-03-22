//! A trait for time instants.

use std::fmt::Debug;
use std::ops::Add;
use std::ops::AddAssign;
use std::ops::Sub;
use std::ops::SubAssign;
use std::panic::RefUnwindSafe;
use std::panic::UnwindSafe;
use std::time::Duration;

use crate::OptionalSend;
use crate::OptionalSync;

/// A measurement of a monotonically non-decreasing clock.
pub trait Instant:
    Add<Duration, Output = Self>
    + AddAssign<Duration>
    + Clone
    + Copy
    + Debug
    + Eq
    + Ord
    + PartialEq
    + PartialOrd
    + RefUnwindSafe
    + OptionalSend
    + Sub<Duration, Output = Self>
    + Sub<Self, Output = Duration>
    + SubAssign<Duration>
    + OptionalSync
    + Unpin
    + UnwindSafe
    + 'static
{
    /// Return the current instant.
    fn now() -> Self;

    /// Return the amount of time since the instant.
    ///
    /// The returned duration is guaranteed to be non-negative.
    fn elapsed(&self) -> Duration {
        let now = Self::now();
        if now > *self {
            now - *self
        } else {
            Duration::from_secs(0)
        }
    }
}

pub type TokioInstant = tokio::time::Instant;

impl Instant for tokio::time::Instant {
    #[inline]
    fn now() -> Self {
        tokio::time::Instant::now()
    }
}
