use std::fmt;

/// Implement `Display` for `Option<T>` if T is `Display`.
///
/// It outputs a literal string `"None"` if it is None, otherwise it invokes the Display
/// implementation for T.
pub(crate) struct DisplayOption<'a, T: fmt::Display>(pub &'a Option<T>);

impl<T: fmt::Display> fmt::Display for DisplayOption<'_, T> {
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
    fn display(&self) -> DisplayOption<'_, T> {
        DisplayOption(self)
    }
}
