//! Virtual instants.

use std::fmt;
use std::ops::Add;
use std::ops::AddAssign;
use std::ops::Sub;
use std::ops::SubAssign;
use std::time::Duration;

use openraft_rt::instant;

use crate::executor;

/// A point on the simulated clock, in nanoseconds.
///
/// [`now()`](instant::Instant::now) reads the runtime installed by `block_on` and panics outside
/// it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SimInstant(pub(crate) u64);

impl SimInstant {
    /// The current virtual time, or `None` outside `block_on`. For code that may run either way,
    /// such as a log formatter.
    pub fn try_now() -> Option<Self> {
        executor::try_current().map(|shared| SimInstant(shared.lock().now))
    }

    /// Nanoseconds since the start of the virtual clock.
    pub fn elapsed_since_start(self) -> std::time::Duration {
        std::time::Duration::from_nanos(self.0 - executor::EPOCH_NANOS.min(self.0))
    }

    pub(crate) fn nanos(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for SimInstant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SimInstant({}ns)", self.0 - executor::EPOCH_NANOS.min(self.0))
    }
}

fn duration_nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).expect("rt-sim: duration does not fit in u64 nanoseconds")
}

impl Add<Duration> for SimInstant {
    type Output = Self;

    fn add(self, rhs: Duration) -> Self {
        SimInstant(self.0.checked_add(duration_nanos(rhs)).expect("rt-sim: instant overflow"))
    }
}

impl AddAssign<Duration> for SimInstant {
    fn add_assign(&mut self, rhs: Duration) {
        *self = *self + rhs;
    }
}

impl Sub<Duration> for SimInstant {
    type Output = Self;

    fn sub(self, rhs: Duration) -> Self {
        SimInstant(self.0.checked_sub(duration_nanos(rhs)).expect("rt-sim: instant underflow"))
    }
}

impl SubAssign<Duration> for SimInstant {
    fn sub_assign(&mut self, rhs: Duration) {
        *self = *self - rhs;
    }
}

impl Sub<SimInstant> for SimInstant {
    type Output = Duration;

    /// Saturates at zero, like `std::time::Instant`.
    fn sub(self, rhs: SimInstant) -> Duration {
        Duration::from_nanos(self.0.saturating_sub(rhs.0))
    }
}

impl instant::Instant for SimInstant {
    #[track_caller]
    fn now() -> Self {
        SimInstant(executor::current().lock().now)
    }
}
