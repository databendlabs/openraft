use std::fmt;

/// Implement `Display` for `Result<T,E>` if T and E are `Display`.
///
/// It outputs `"Ok(...)"` or `"Err(...)"`.
pub(crate) struct DisplayResult<'a, T: fmt::Display, E: fmt::Display>(pub &'a Result<T, E>);

impl<T, E> fmt::Display for DisplayResult<'_, T, E>
where
    T: fmt::Display,
    E: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Ok(ok) => {
                write!(f, "Ok({})", ok)
            }
            Err(err) => {
                write!(f, "Err({})", err)
            }
        }
    }
}

pub(crate) trait DisplayResultExt<'a, T: fmt::Display, E: fmt::Display> {
    fn display(&'a self) -> DisplayResult<'a, T, E>;
}

impl<T, E> DisplayResultExt<'_, T, E> for Result<T, E>
where
    T: fmt::Display,
    E: fmt::Display,
{
    fn display(&self) -> DisplayResult<'_, T, E> {
        DisplayResult(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_result() {
        let result: Result<i32, &str> = Ok(42);
        let display_result = DisplayResult(&result);
        assert_eq!(format!("{}", display_result), "Ok(42)");

        let result: Result<i32, &str> = Err("error");
        let display_result = DisplayResult(&result);
        assert_eq!(format!("{}", display_result), "Err(error)");
    }
}
