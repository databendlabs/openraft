//! Instant wrapper type and its trait impl.

use std::ops::Add;
use std::ops::AddAssign;
use std::ops::Sub;
use std::ops::SubAssign;
use std::time::Duration;

use openraft_rt::instant;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct MonoioInstant(pub(crate) monoio::time::Instant);

impl Add<Duration> for MonoioInstant {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Duration) -> Self::Output {
        Self(self.0.add(rhs))
    }
}

impl AddAssign<Duration> for MonoioInstant {
    #[inline]
    fn add_assign(&mut self, rhs: Duration) {
        self.0.add_assign(rhs)
    }
}

impl Sub<Duration> for MonoioInstant {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Duration) -> Self::Output {
        Self(self.0.sub(rhs))
    }
}

impl Sub<Self> for MonoioInstant {
    type Output = Duration;

    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        self.0.sub(rhs.0)
    }
}

impl SubAssign<Duration> for MonoioInstant {
    #[inline]
    fn sub_assign(&mut self, rhs: Duration) {
        self.0.sub_assign(rhs)
    }
}

impl instant::Instant for MonoioInstant {
    #[inline]
    fn now() -> Self {
        let inner = monoio::time::Instant::now();
        Self(inner)
    }

    #[inline]
    fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }
}
