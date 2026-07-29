//! Utilities for converting `Result<T, Infallible>` into `T`.

use crate::errors::Infallible;

/// Convert `Result<T, E>` to `T`, if `E` is a `never` type.
pub(crate) fn into_ok<T, E>(result: Result<T, E>) -> T
where E: Into<Infallible> {
    match result {
        Ok(t) => t,
        Err(e) => {
            // NOTE: `allow` required because of buggy reachability detection by rust compiler
            #[allow(unreachable_code)]
            match e.into() {}
        }
    }
}
