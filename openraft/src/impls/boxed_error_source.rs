//! A boxed error wrapper for smaller error type sizes.

use std::error::Error;
use std::fmt;

use anyerror::AnyError;

use crate::errors::ErrorSource;

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

    /// Whether the wrapped error stores a backtrace.
    fn has_backtrace(&self) -> bool {
        self.inner.backtrace().is_some()
    }

    /// Write the backtrace stored in the wrapped error.
    fn fmt_backtrace(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(bt) = self.inner.backtrace() {
            write!(f, "{}", bt)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::BoxedErrorSource;
    use crate::errors::BacktraceDisplay;
    use crate::errors::ErrorSource;

    /// The backtrace is the one stored in the wrapped error, also when it came over the wire.
    #[cfg(feature = "serde")]
    #[test]
    fn test_reports_the_stored_backtrace_of_the_wrapped_error() {
        let json = serde_json::json!({"inner": {
            "typ": null, "msg": "read", "source": null, "context": [], "backtrace": "stored-marker"
        }});
        let err: BoxedErrorSource = serde_json::from_value(json).unwrap();

        assert!(err.has_backtrace());
        assert_eq!("stored-marker", BacktraceDisplay(&err).to_string());
    }

    /// A wrapped error without a stored backtrace reports none, also when backtrace capture is
    /// enabled.
    #[test]
    fn test_reports_no_backtrace_when_the_wrapped_error_has_none() {
        let err = BoxedErrorSource::from_string("read");

        assert!(!err.has_backtrace());
        assert_eq!("", BacktraceDisplay(&err).to_string());
    }
}
