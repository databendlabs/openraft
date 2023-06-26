use std::fmt::Debug;
use std::ops::Add;
use std::ops::AddAssign;
use std::ops::Sub;
use std::ops::SubAssign;
use std::panic::RefUnwindSafe;
use std::panic::UnwindSafe;
use std::time::Duration;

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
    + Send
    + Sub<Duration, Output = Self>
    + Sub<Self, Output = Duration>
    + SubAssign<Duration>
    + Sync
    + Unpin
    + UnwindSafe
    + 'static
{
    /// Return the current instant.
    fn now() -> Self;
}

pub type TokioInstant = tokio::time::Instant;

impl Instant for tokio::time::Instant {
    #[inline]
    fn now() -> Self {
        tokio::time::Instant::now()
    }
}

impl Instant for std::time::Instant {
    #[inline]
    fn now() -> Self {
        Self::now()
    }
}
