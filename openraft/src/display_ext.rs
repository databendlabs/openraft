//! Implement [`fmt::Display`] for types such as `Option<T>` and slice `&[T]`.

use std::fmt;

use chrono::DateTime;
use chrono::Local;
use chrono::Utc;

use crate::Instant;

/// Implement `Display` for `Option<T>` if T is `Display`.
///
/// It outputs a literal string `"None"` if it is None. Otherwise it invokes the Display
/// implementation for T.
pub(crate) struct DisplayOption<'a, T: fmt::Display>(pub &'a Option<T>);

impl<'a, T: fmt::Display> fmt::Display for DisplayOption<'a, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            None => {
                write!(f, "None")
            }
            Some(x) => x.fmt(f),
        }
    }
}

pub(crate) trait DisplayOptionExt<'a, T: fmt::Display> {
    fn display(&'a self) -> DisplayOption<'a, T>;
}

impl<T> DisplayOptionExt<'_, T> for Option<T>
where T: fmt::Display
{
    fn display(&self) -> DisplayOption<T> {
        DisplayOption(self)
    }
}

/// Implement `Display` for `&[T]` if T is `Display`.
///
/// It outputs at most `MAX` elements, excluding those from the 5th to the second-to-last one:
/// - `DisplaySlice(&[1,2,3,4,5,6])` outputs: `"[1,2,3,4,...,6]"`.
pub(crate) struct DisplaySlice<'a, T: fmt::Display, const MAX: usize = 5>(pub &'a [T]);

impl<'a, T: fmt::Display, const MAX: usize> fmt::Display for DisplaySlice<'a, T, MAX> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let slice = self.0;
        let len = slice.len();

        write!(f, "[")?;

        if len > MAX {
            for (i, t) in slice[..(MAX - 1)].iter().enumerate() {
                if i > 0 {
                    write!(f, ",")?;
                }

                write!(f, "{}", t)?;
            }

            write!(f, ",..,")?;
            write!(f, "{}", slice.last().unwrap())?;
        } else {
            for (i, t) in slice.iter().enumerate() {
                if i > 0 {
                    write!(f, ",")?;
                }

                write!(f, "{}", t)?;
            }
        }

        write!(f, "]")
    }
}

pub(crate) trait DisplaySliceExt<'a, T: fmt::Display> {
    fn display(&'a self) -> DisplaySlice<'a, T>;
}

impl<T> DisplaySliceExt<'_, T> for [T]
where T: fmt::Display
{
    fn display(&self) -> DisplaySlice<T> {
        DisplaySlice(self)
    }
}

/// Display `Instant` in human readable format.
pub(crate) struct DisplayInstant<'a, T, const SIMPLE: bool = true, const LOCAL: bool = true>(pub &'a T);

impl<'a, T, const SIMPLE: bool, const LOCAL: bool> DisplayInstant<'a, T, SIMPLE, LOCAL> {
    /// Display `Instant` in full format: with date and timezone: the format is
    /// "%Y-%m-%dT%H:%M:%S%.6fZ%z"
    #[allow(dead_code)]
    pub fn full(self) -> DisplayInstant<'a, T, false, LOCAL> {
        DisplayInstant(self.0)
    }

    /// Display `Instant` in simple format: without date and timezone: the format is "%H:%M:%S%.6f"
    #[allow(dead_code)]
    pub fn simple(self) -> DisplayInstant<'a, T, true, LOCAL> {
        DisplayInstant(self.0)
    }

    /// Display `Instant` in local timezone.
    #[allow(dead_code)]
    pub fn local(self) -> DisplayInstant<'a, T, SIMPLE, true> {
        DisplayInstant(self.0)
    }

    /// Display `Instant` in UTC timezone.
    #[allow(dead_code)]
    pub fn utc(self) -> DisplayInstant<'a, T, SIMPLE, false> {
        DisplayInstant(self.0)
    }
}

impl<'a, T, const SIMPLE: bool, const LOCAL: bool> fmt::Display for DisplayInstant<'a, T, SIMPLE, LOCAL>
where T: Instant
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Convert Instant to SystemTime
        let sys_t = {
            let sys_now = std::time::SystemTime::now();
            let now = T::now();

            if &now >= self.0 {
                let d = now - *self.0;
                sys_now - d
            } else {
                let d = *self.0 - now;
                sys_now + d
            }
        };

        if LOCAL {
            let datetime: DateTime<Local> = sys_t.into();

            if SIMPLE {
                write!(f, "{}", datetime.format("%H:%M:%S%.6f"))
            } else {
                write!(f, "{}", datetime.format("%Y-%m-%dT%H:%M:%S%.6fZ%z"))
            }
        } else {
            let datetime: DateTime<Utc> = sys_t.into();

            if SIMPLE {
                write!(f, "{}", datetime.format("%H:%M:%S%.6f"))
            } else {
                write!(f, "{}", datetime.format("%Y-%m-%dT%H:%M:%S%.6fZ%z"))
            }
        }
    }
}

pub(crate) trait DisplayInstantExt<'a, T> {
    fn display(&'a self) -> DisplayInstant<'a, T, true>;
}

impl<T> DisplayInstantExt<'_, T> for T
where T: Instant
{
    fn display(&self) -> DisplayInstant<T, true> {
        DisplayInstant(self)
    }
}

#[cfg(test)]
mod tests {
    use crate::display_ext::DisplayInstantExt;
    use crate::display_ext::DisplaySlice;
    use crate::TokioInstant;

    #[test]
    fn test_display_slice() {
        let a = vec![1, 2, 3, 4];
        assert_eq!("[1,2,3,4]", DisplaySlice::<_>(&a).to_string());

        let a = vec![1, 2, 3, 4, 5];
        assert_eq!("[1,2,3,4,5]", DisplaySlice::<_>(&a).to_string());

        let a = vec![1, 2, 3, 4, 5, 6];
        assert_eq!("[1,2,3,4,..,6]", DisplaySlice::<_>(&a).to_string());

        let a = vec![1, 2, 3, 4, 5, 6, 7];
        assert_eq!("[1,2,3,4,..,7]", DisplaySlice::<_>(&a).to_string());

        let a = vec![1, 2, 3, 4, 5, 6, 7];
        assert_eq!("[1,..,7]", DisplaySlice::<_, 2>(&a).to_string());
    }

    /// Check the result by a human.
    #[test]
    fn test_display_instant() -> anyhow::Result<()> {
        let now = TokioInstant::now();
        println!("now: {}", now.display());
        println!("now: {}", now.display().full());
        println!("now: {}", now.display().full().simple());
        println!("now: {}", now.display().utc());
        println!("now: {}", now.display().utc().local());
        println!("now: {}", now.display().utc().full());
        Ok(())
    }
}
