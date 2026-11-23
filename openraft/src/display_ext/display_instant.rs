use std::fmt;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use crate::Instant;

/// Display `Instant` as a UTC wall-clock time.
pub(crate) struct DisplayInstant<'a, T, const SIMPLE: bool = true>(pub &'a T);

impl<'a, T, const SIMPLE: bool> DisplayInstant<'a, T, SIMPLE> {
    /// Display `Instant` in full format, with date: `yyyy-mm-ddTHH:MM:SS.ffffffZ`.
    #[allow(dead_code)]
    pub fn full(self) -> DisplayInstant<'a, T, false> {
        DisplayInstant(self.0)
    }

    /// Display `Instant` in simple format, without date: `HH:MM:SS.ffffff`.
    #[allow(dead_code)]
    pub fn simple(self) -> DisplayInstant<'a, T, true> {
        DisplayInstant(self.0)
    }
}

impl<T, const SIMPLE: bool> fmt::Display for DisplayInstant<'_, T, SIMPLE>
where T: Instant
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Convert Instant to SystemTime
        let sys_t = {
            let sys_now = SystemTime::now();
            let now = T::now();

            if &now >= self.0 {
                let d = now - *self.0;
                sys_now - d
            } else {
                let d = *self.0 - now;
                sys_now + d
            }
        };

        DisplayUtc::<SIMPLE>(sys_t).fmt(f)
    }
}

/// Display a `SystemTime` as a UTC time, with the date unless `SIMPLE`.
struct DisplayUtc<const SIMPLE: bool>(SystemTime);

impl<const SIMPLE: bool> fmt::Display for DisplayUtc<SIMPLE> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (secs, micros) = unix_secs_micros(self.0);

        let days = secs.div_euclid(86_400);
        let secs_of_day = secs.rem_euclid(86_400);
        let (hour, minute, second) = (secs_of_day / 3600, secs_of_day % 3600 / 60, secs_of_day % 60);

        if !SIMPLE {
            let (year, month, day) = civil_from_days(days);
            write!(f, "{year:04}-{month:02}-{day:02}T")?;
        }

        write!(f, "{hour:02}:{minute:02}:{second:02}.{micros:06}")?;

        if !SIMPLE {
            write!(f, "Z")?;
        }
        Ok(())
    }
}

/// Split a `SystemTime` into whole seconds since the Unix epoch and the microseconds within that
/// second.
///
/// Seconds are negative before the epoch; microseconds are always in `[0, 1_000_000)`.
fn unix_secs_micros(t: SystemTime) -> (i64, u32) {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => (d.as_secs() as i64, d.subsec_micros()),
        Err(e) => {
            // `t` lies `d` before the epoch. Borrow a whole second for any fraction of a second
            // first, and truncate only the fraction within that second, so that `t` stays in its
            // own second and microsecond.
            let d = e.duration();
            let secs = -(d.as_secs() as i64);
            match d.subsec_nanos() {
                0 => (secs, 0),
                nanos => (secs - 1, (1_000_000_000 - nanos) / 1_000),
            }
        }
    }
}

/// Convert days since 1970-01-01 to a proleptic Gregorian `(year, month, day)`.
///
/// Howard Hinnant's `civil_from_days`: <https://howardhinnant.github.io/date_algorithms.html>
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    // Shift the epoch to 0000-03-01, so that the leap day is the last day of a year.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    // Month counted from March: 0 is March, 11 is February.
    let mp = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

pub(crate) trait DisplayInstantExt<'a, T> {
    fn display(&'a self) -> DisplayInstant<'a, T, true>;
}

impl<T> DisplayInstantExt<'_, T> for T
where T: Instant
{
    fn display(&self) -> DisplayInstant<'_, T, true> {
        DisplayInstant(self)
    }
}

#[cfg(test)]
mod display_instant_test;
