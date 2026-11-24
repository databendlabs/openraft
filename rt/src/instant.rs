//! A trait for time instants.

use std::fmt::Debug;
use std::ops::Add;
use std::ops::AddAssign;
use std::ops::Sub;
use std::ops::SubAssign;
use std::panic::RefUnwindSafe;
use std::panic::UnwindSafe;
use std::time::Duration;

use openraft_macros::since;

use crate::OptionalSend;
use crate::OptionalSync;

/// A measurement of a monotonically non-decreasing clock.
///
/// Each runtime supplies its own clock, and reading it can require an active runtime context: the
/// simulated clock of `openraft-rt-sim` exists only inside its `block_on`. Measure time only
/// between instants of the same clock.
///
/// A clock represents a limited range of instants. Shifting an instant out of that range with `+`,
/// `-`, `+=` or `-=` can panic, while [`Self::checked_add`] and [`Self::checked_sub`] return
/// `None`.
#[since(
    version = "0.10.2",
    change = "add the required methods `checked_add` and `checked_sub`"
)]
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
    ///
    /// A runtime whose clock needs an active context, such as a simulated clock, panics when this
    /// is called outside that context.
    #[track_caller]
    fn now() -> Self;

    /// Return the instant `duration` after this one, or `None` if the clock cannot represent it.
    ///
    /// It never panics: an implementation checks the range itself, such as with the `checked_add`
    /// of the instant type it wraps, instead of calling an operator that can panic.
    #[since(version = "0.10.2")]
    fn checked_add(&self, duration: Duration) -> Option<Self>;

    /// Return the instant `duration` before this one, or `None` if the clock cannot represent it.
    ///
    /// Like [`Self::checked_add`], it never panics.
    #[since(version = "0.10.2")]
    fn checked_sub(&self, duration: Duration) -> Option<Self>;

    /// Return the amount of time since the instant, or zero if the instant is in the future.
    ///
    /// This reads the current clock, so it has the same context requirement as [`Self::now`]. A
    /// caller that already has two instants of the same clock can use
    /// [`Self::saturating_duration_since`], which does not read the clock.
    #[track_caller]
    fn elapsed(&self) -> Duration {
        let now = Self::now();
        if now > *self {
            now - *self
        } else {
            Duration::from_secs(0)
        }
    }

    /// Returns the amount of time elapsed from another instant to this one, or zero duration if
    /// that instant is later than this one.
    fn saturating_duration_since(&self, earlier: Self) -> Duration {
        if *self > earlier {
            *self - earlier
        } else {
            Duration::from_secs(0)
        }
    }
}

impl Instant for std::time::Instant {
    fn now() -> Self {
        std::time::Instant::now()
    }

    fn checked_add(&self, duration: Duration) -> Option<Self> {
        std::time::Instant::checked_add(self, duration)
    }

    fn checked_sub(&self, duration: Duration) -> Option<Self> {
        std::time::Instant::checked_sub(self, duration)
    }
}
