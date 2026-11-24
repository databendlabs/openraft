use std::ops::Add;
use std::ops::AddAssign;
use std::ops::Sub;
use std::ops::SubAssign;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use super::SerdeInstant;
use crate::Instant;
use crate::engine::testing::UTConfig;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::SerdeInstantOf;

const DAY: u64 = 86_400;

/// An instant of a clock that counts a `Duration` from its start, and whose `now()` is always
/// `NOW_SECS` seconds. It cannot represent a time before its start or past `Duration::MAX`, and its
/// operators panic there, as those of a platform clock do.
#[derive(Debug, Clone, Copy)]
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct TestInstant<const NOW_SECS: u64>(Duration);

/// A clock that started a day ago.
type StartedDayAgo = TestInstant<DAY>;

/// A clock that cannot represent a time more than a day from now.
type EndsInDay = TestInstant<{ u64::MAX - DAY }>;

impl<const NOW_SECS: u64> Add<Duration> for TestInstant<NOW_SECS> {
    type Output = Self;

    fn add(self, rhs: Duration) -> Self {
        Self(self.0 + rhs)
    }
}

impl<const NOW_SECS: u64> AddAssign<Duration> for TestInstant<NOW_SECS> {
    fn add_assign(&mut self, rhs: Duration) {
        self.0 += rhs;
    }
}

impl<const NOW_SECS: u64> Sub<Duration> for TestInstant<NOW_SECS> {
    type Output = Self;

    fn sub(self, rhs: Duration) -> Self {
        Self(self.0 - rhs)
    }
}

impl<const NOW_SECS: u64> Sub for TestInstant<NOW_SECS> {
    type Output = Duration;

    fn sub(self, rhs: Self) -> Duration {
        self.0.saturating_sub(rhs.0)
    }
}

impl<const NOW_SECS: u64> SubAssign<Duration> for TestInstant<NOW_SECS> {
    fn sub_assign(&mut self, rhs: Duration) {
        self.0 -= rhs;
    }
}

impl<const NOW_SECS: u64> Instant for TestInstant<NOW_SECS> {
    fn now() -> Self {
        Self(Duration::from_secs(NOW_SECS))
    }

    fn checked_add(&self, duration: Duration) -> Option<Self> {
        self.0.checked_add(duration).map(Self)
    }

    fn checked_sub(&self, duration: Duration) -> Option<Self> {
        self.0.checked_sub(duration).map(Self)
    }
}

#[test]
fn test_serde_instant() {
    let now = UTConfig::<()>::now();
    let serde_instant = SerdeInstant::new(now);
    let json = serde_json::to_string(&serde_instant).unwrap();
    println!("json: {}", json);
    println!("Now: {:?}", now);

    let deserialized: SerdeInstantOf<UTConfig> = serde_json::from_str(&json).unwrap();
    println!("Des: {:?}", *deserialized);

    // Convert Instant to SerdeInstant is inaccurate.
    if now > *deserialized {
        assert!((now - *deserialized) < Duration::from_millis(5));
    } else {
        assert!((*deserialized - now) < Duration::from_millis(5));
    }

    // Test serialization format

    let nano = "1721829051211301916";
    let deserialized: SerdeInstantOf<UTConfig> = serde_json::from_str(nano).unwrap();
    let serialized = serde_json::to_string(&deserialized).unwrap();

    assert_eq!(
        nano[0..nano.len() - 6],
        serialized[0..serialized.len() - 6],
        "compare up to milli seconds: {}",
        &nano[0..nano.len() - 6]
    );
}

/// Decoding returns an error for a time that the runtime's clock cannot represent, before or after
/// now.
#[test]
fn test_deserialize_out_of_clock_range() {
    let now_nanos = u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()).unwrap();
    let is_clock_range_error = |e: &serde_json::Error| e.to_string().contains("of the runtime's clock");

    tracing::info!(
        now_nanos,
        "a clock that started a day ago decodes now, but cannot represent the Unix epoch"
    );
    {
        let decoded = serde_json::from_str::<SerdeInstant<StartedDayAgo>>(&now_nanos.to_string());
        assert!(decoded.is_ok(), "{decoded:?}");

        let err = serde_json::from_str::<SerdeInstant<StartedDayAgo>>("0").unwrap_err();
        assert!(is_clock_range_error(&err), "{err}");
    }

    tracing::info!(
        now_nanos,
        "a clock that ends in a day decodes now, but cannot represent u64::MAX ns, in 2554"
    );
    {
        let decoded = serde_json::from_str::<SerdeInstant<EndsInDay>>(&now_nanos.to_string());
        assert!(decoded.is_ok(), "{decoded:?}");

        let err = serde_json::from_str::<SerdeInstant<EndsInDay>>(&u64::MAX.to_string()).unwrap_err();
        assert!(is_clock_range_error(&err), "{err}");
    }
}

/// Encoding returns an error for an instant whose wall-clock time `SystemTime` cannot represent,
/// before or after now.
#[test]
fn test_serialize_out_of_system_time_range() {
    tracing::info!("now encodes, but SystemTime cannot represent an instant about Duration::MAX after it");
    {
        let now = SerdeInstant::new(StartedDayAgo::now());
        assert!(serde_json::to_string(&now).is_ok());

        let far: SerdeInstant<StartedDayAgo> = SerdeInstant::new(TestInstant(Duration::MAX));
        let err = serde_json::to_string(&far).unwrap_err();
        assert_eq!(err.to_string(), "time out of range");
    }

    tracing::info!("now encodes, but SystemTime cannot represent an instant about Duration::MAX before it");
    {
        let now = SerdeInstant::new(EndsInDay::now());
        assert!(serde_json::to_string(&now).is_ok());

        let far: SerdeInstant<EndsInDay> = SerdeInstant::new(TestInstant(Duration::ZERO));
        let err = serde_json::to_string(&far).unwrap_err();
        assert_eq!(err.to_string(), "time out of range");
    }
}
