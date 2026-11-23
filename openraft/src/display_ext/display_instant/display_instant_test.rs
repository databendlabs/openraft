use std::fmt;
use std::fmt::Write;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use super::DisplayUtc;
use super::civil_from_days;
use super::unix_secs_micros;
use crate::display_ext::DisplayInstantExt;
use crate::engine::testing::UTConfig;
use crate::type_config::TypeConfigExt;

/// A past, the current and a future instant show the wall-clock time they stand for.
///
/// That time lies between the wall-clock times read before and after formatting, moved by the
/// instant's offset from the runtime clock. The bounds allow for the time between the clock reads
/// and for one second of clock adjustment; each offset is far larger, so a display that ignored
/// or reversed it would fall outside them.
#[test]
fn test_display_instant_wall_clock() {
    let allowance = Duration::from_secs(1);

    // The offset of the instant from the runtime clock, and whether it lies in the future.
    let cases = [
        (Duration::from_secs(60), false),
        (Duration::ZERO, false),
        (Duration::from_secs(86_400), true),
    ];

    for (offset, future) in cases {
        let before = SystemTime::now();
        let base = UTConfig::<()>::now();
        let instant = if future { base + offset } else { base - offset };
        let full = instant.display().full().to_string();
        let elapsed = UTConfig::<()>::now() - base;
        let after = SystemTime::now();

        let shift = |t: SystemTime| if future { t + offset } else { t - offset };
        // The full format has a fixed width, so its text sorts as the time it shows.
        let lo = DisplayUtc::<false>(shift(before) - elapsed - allowance).to_string();
        let hi = DisplayUtc::<false>(shift(after) + allowance).to_string();
        assert!(
            lo <= full && full <= hi,
            "offset {offset:?}, future: {future}: {full} is not in [{lo}, {hi}]"
        );
    }
}

/// `display()` starts in the simple format, and `full()` and `simple()` select a format whatever
/// was selected before.
#[test]
fn test_display_instant_format_builders() {
    let shape = |s: String| s.chars().map(|c| if c.is_ascii_digit() { '9' } else { c }).collect::<String>();
    let full = "9999-99-99T99:99:99.999999Z";
    let simple = "99:99:99.999999";

    let now = UTConfig::<()>::now();
    assert_eq!(simple, shape(now.display().to_string()));
    assert_eq!(full, shape(now.display().full().to_string()));
    assert_eq!(simple, shape(now.display().simple().to_string()));
    assert_eq!(simple, shape(now.display().full().simple().to_string()));
    assert_eq!(full, shape(now.display().simple().full().to_string()));
}

/// An error of the writer is returned, in both formats.
#[test]
fn test_display_instant_writer_error() {
    struct Reject;

    impl Write for Reject {
        fn write_str(&mut self, _: &str) -> fmt::Result {
            Err(fmt::Error)
        }
    }

    let now = UTConfig::<()>::now();
    assert_eq!(Err(fmt::Error), write!(Reject, "{}", now.display()));
    assert_eq!(Err(fmt::Error), write!(Reject, "{}", now.display().full()));
}

#[test]
fn test_display_utc() {
    let t = UNIX_EPOCH + Duration::new(1_721_829_051, 211_301_916);
    assert_eq!("2024-07-24T13:50:51.211301Z", DisplayUtc::<false>(t).to_string());
    assert_eq!("13:50:51.211301", DisplayUtc::<true>(t).to_string());

    assert_eq!(
        "1970-01-01T00:00:00.000000Z",
        DisplayUtc::<false>(UNIX_EPOCH).to_string()
    );

    let before_epoch = UNIX_EPOCH - Duration::from_micros(1);
    assert_eq!(
        "1969-12-31T23:59:59.999999Z",
        DisplayUtc::<false>(before_epoch).to_string()
    );
}

/// Every time stays in its own second and microsecond, on both sides of the epoch.
#[test]
fn test_unix_secs_micros() {
    let nanos = |n: u64| Duration::from_nanos(n);

    let after = [
        (1, (0, 0)),
        (999, (0, 0)),
        (1_000, (0, 1)),
        (1_001, (0, 1)),
        (999_999_999, (0, 999_999)),
        (1_000_000_000, (1, 0)),
        (1_000_000_001, (1, 0)),
    ];
    for (n, want) in after {
        assert_eq!(want, unix_secs_micros(UNIX_EPOCH + nanos(n)), "{n} ns after the epoch");
    }

    let before = [
        (1, (-1, 999_999)),
        (999, (-1, 999_999)),
        (1_000, (-1, 999_999)),
        (1_001, (-1, 999_998)),
        (999_999_999, (-1, 0)),
        (1_000_000_000, (-1, 0)),
        (1_000_000_001, (-2, 999_999)),
    ];
    for (n, want) in before {
        assert_eq!(want, unix_secs_micros(UNIX_EPOCH - nanos(n)), "{n} ns before the epoch");
    }
}

#[test]
fn test_display_utc_around_the_epoch() {
    let cases = [
        (UNIX_EPOCH - Duration::from_nanos(1), "1969-12-31T23:59:59.999999Z"),
        (UNIX_EPOCH - Duration::from_nanos(1_001), "1969-12-31T23:59:59.999998Z"),
        (
            UNIX_EPOCH - Duration::from_nanos(999_999_999),
            "1969-12-31T23:59:59.000000Z",
        ),
        (
            UNIX_EPOCH - Duration::from_nanos(1_000_000_001),
            "1969-12-31T23:59:58.999999Z",
        ),
        (UNIX_EPOCH + Duration::from_nanos(999), "1970-01-01T00:00:00.000000Z"),
        (
            UNIX_EPOCH + Duration::from_nanos(1_000_000_001),
            "1970-01-01T00:00:01.000000Z",
        ),
    ];
    for (t, full) in cases {
        assert_eq!(full, DisplayUtc::<false>(t).to_string());
        let simple = &full["yyyy-mm-ddT".len()..full.len() - 1];
        assert_eq!(simple, DisplayUtc::<true>(t).to_string());
    }
}

#[test]
fn test_civil_from_days() {
    assert_eq!((1970, 1, 1), civil_from_days(0));
    assert_eq!((1969, 12, 31), civil_from_days(-1));
    assert_eq!((2000, 2, 29), civil_from_days(11_016));
    assert_eq!((2000, 3, 1), civil_from_days(11_017));
    assert_eq!((2024, 1, 1), civil_from_days(19_723));
    assert_eq!((2024, 7, 24), civil_from_days(19_928));
}
