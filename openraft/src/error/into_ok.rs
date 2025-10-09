//! Utilities for converting `Result<T, Infallible>` into `T`.

use crate::error::Infallible;

/// Trait to convert `Result<T, E>` to `T`, if `E` is a `never` type.
pub(crate) trait UnwrapInfallible<T> {
    fn into_ok(self) -> T;
}

impl<T, E> UnwrapInfallible<T> for Result<T, E>
where E: Into<Infallible>
{
    fn into_ok(self) -> T {
        match self {
            Ok(t) => t,
            Err(e) => {
                // NOTE: `allow` required because of buggy reachability detection by rust compiler
                #[allow(unreachable_code)]
                match e.into() {}
            }
        }
    }
}

/// Convert `Result<T, E>` to `T`, if `E` is a `never` type.
pub(crate) fn into_ok<T, E>(result: Result<T, E>) -> T
where E: Into<Infallible> {
    UnwrapInfallible::into_ok(result)
}
