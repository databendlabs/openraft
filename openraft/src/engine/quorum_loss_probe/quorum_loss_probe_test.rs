use std::ops::Add;
use std::ops::AddAssign;
use std::ops::Sub;
use std::ops::SubAssign;
use std::time::Duration;

use super::QuorumLossProbe;
use crate::engine::testing::UTConfig;
use crate::rt::Instant;
use crate::type_config::TypeConfigExt;

/// An instant that is a `Duration` since a fixed origin, so a test can reach spans that a
/// platform clock cannot.
#[derive(Debug, Clone, Copy)]
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct SpanInstant(Duration);

impl Add<Duration> for SpanInstant {
    type Output = Self;

    fn add(self, rhs: Duration) -> Self {
        Self(self.0 + rhs)
    }
}

impl AddAssign<Duration> for SpanInstant {
    fn add_assign(&mut self, rhs: Duration) {
        self.0 += rhs;
    }
}

impl Sub<Duration> for SpanInstant {
    type Output = Self;

    fn sub(self, rhs: Duration) -> Self {
        Self(self.0 - rhs)
    }
}

impl Sub for SpanInstant {
    type Output = Duration;

    fn sub(self, rhs: Self) -> Duration {
        self.0 - rhs.0
    }
}

impl SubAssign<Duration> for SpanInstant {
    fn sub_assign(&mut self, rhs: Duration) {
        self.0 -= rhs;
    }
}

impl Instant for SpanInstant {
    fn now() -> Self {
        Self(Duration::ZERO)
    }

    fn checked_add(&self, duration: Duration) -> Option<Self> {
        self.0.checked_add(duration).map(Self)
    }

    fn checked_sub(&self, duration: Duration) -> Option<Self> {
        self.0.checked_sub(duration).map(Self)
    }
}

#[test]
fn test_quiet_for() {
    let ms = Duration::from_millis;
    let t = UTConfig::<()>::now();
    let probe = QuorumLossProbe {
        quorum_acked_at: t,
        leader_lease: ms(300),
        interval: ms(600),
    };

    assert_eq!(None, probe.quiet_for(t));
    assert_eq!(None, probe.quiet_for(t + ms(299)));
    assert_eq!(Some(ms(600)), probe.quiet_for(t + ms(300)));
    assert_eq!(Some(ms(1)), probe.quiet_for(t + ms(899)));
    assert_eq!(None, probe.quiet_for(t + ms(900)));
    assert_eq!(None, probe.quiet_for(t + ms(1_499)));
    assert_eq!(Some(ms(400)), probe.quiet_for(t + ms(1_700)));
}

/// The phase and its remaining time just before, at and just after the end of the lease and of
/// each period.
#[test]
fn test_phase() {
    let ms = Duration::from_millis;
    let t = UTConfig::<()>::now();
    let probe = QuorumLossProbe {
        quorum_acked_at: t,
        leader_lease: ms(300),
        interval: ms(600),
    };

    for (elapsed, expected) in [
        (0, (false, ms(300))),
        (299, (false, ms(1))),
        (300, (true, ms(600))),
        (301, (true, ms(599))),
        (899, (true, ms(1))),
        (900, (false, ms(600))),
        (901, (false, ms(599))),
        (1_499, (false, ms(1))),
        (1_500, (true, ms(600))),
        (1_501, (true, ms(599))),
    ] {
        let actual = probe.phase(t + ms(elapsed));
        assert_eq!(expected, actual, "elapsed: {elapsed} ms");
    }
}

/// With an interval and an offset into it of more than `u64::MAX` nanoseconds, about 585 years,
/// the remaining time is exact.
#[test]
fn test_phase_beyond_u64_nanoseconds() {
    let year = Duration::from_secs(365 * 24 * 3600);
    let t = SpanInstant(Duration::ZERO);
    let probe = QuorumLossProbe {
        quorum_acked_at: t,
        leader_lease: Duration::from_secs(1),
        interval: year * 1_000,
    };
    let into_period = year * 600 + Duration::from_millis(500);
    assert!(into_period.as_nanos() > u128::from(u64::MAX));

    let remaining = year * 400 - Duration::from_millis(500);

    let quiet_at = t + Duration::from_secs(1) + into_period;
    assert_eq!((true, remaining), probe.phase(quiet_at));

    let send_at = quiet_at + year * 1_000;
    assert_eq!((false, remaining), probe.phase(send_at));
}
