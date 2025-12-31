//! A boxed error wrapper for smaller error type sizes.

use std::error::Error;
use std::fmt;

use anyerror::AnyError;

use crate::error::ErrorSource;

/// A boxed wrapper around [`AnyError`] for smaller error type sizes.
///
/// This type stores `AnyError` in a `Box`, reducing the size of error types
/// that contain it (like `StorageError`). This is the default `ErrorSource`
/// implementation used by [`declare_raft_types!`](crate::declare_raft_types).
///
/// Use [`AnyError`] directly if you prefer inline storage and don't mind
/// larger error types.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct BoxedErrorSource {
    inner: Box<AnyError>,
}

impl fmt::Display for BoxedErrorSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}

impl Error for BoxedErrorSource {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.inner.source()
    }
}

impl ErrorSource for BoxedErrorSource {
    fn from_error<E: Error + 'static>(error: &E) -> Self {
        Self {
            inner: Box::new(AnyError::new(error)),
        }
    }

    fn from_string(msg: impl ToString) -> Self {
        Self {
            inner: Box::new(AnyError::error(msg)),
        }
    }

    fn has_backtrace(&self) -> bool {
        anyerror::backtrace_str().is_some()
    }

    fn fmt_backtrace(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(bt) = anyerror::backtrace_str() {
            write!(f, "{}", bt)
        } else {
            Ok(())
        }
    }
}
