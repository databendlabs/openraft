//! Configurable error source trait for wrapping arbitrary errors.

use std::error::Error;
use std::fmt;

use anyerror::AnyError;

use crate::OptionalSend;
use crate::OptionalSync;
use crate::base::OptionalSerde;

/// Trait for configurable error wrapper types.
///
/// This trait defines the interface for error types that can wrap arbitrary errors.
/// It allows users to provide custom error implementations with:
/// - Enhanced diagnostic capabilities
/// - In-place error storage (no heap allocation)
/// - Robustness against out-of-memory conditions
///
/// A type config that names none gets [`BoxedErrorSource`] from [`declare_raft_types!`];
/// [`anyerror::AnyError`] implements this trait too. With the `serde` feature, an implementation
/// must also be serializable, as the example derives.
///
/// # Example
///
/// ```
/// use openraft::errors::ErrorSource;
///
/// #[derive(Debug, Clone, PartialEq, Eq)]
/// #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// struct MyError {
///     message: String,
/// }
///
/// impl std::fmt::Display for MyError {
///     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
///         write!(f, "{}", self.message)
///     }
/// }
///
/// impl std::error::Error for MyError {}
///
/// impl ErrorSource for MyError {
///     fn from_error<E: std::error::Error + 'static>(error: &E) -> Self {
///         Self { message: error.to_string() }
///     }
///
///     fn from_string(msg: impl ToString) -> Self {
///         Self { message: msg.to_string() }
///     }
/// }
///
/// let error = MyError::from_string("disk full");
/// assert_eq!("disk full", error.to_string());
/// assert!(!error.has_backtrace());
/// ```
///
/// [`BoxedErrorSource`]: crate::impls::BoxedErrorSource
/// [`declare_raft_types!`]: crate::declare_raft_types
pub trait ErrorSource: Error + Clone + PartialEq + Eq + OptionalSend + OptionalSync + OptionalSerde + 'static {
    /// Create an error from any error type implementing [`Error`].
    fn from_error<E: Error + 'static>(error: &E) -> Self;

    /// Create an error from a string message.
    fn from_string(msg: impl ToString) -> Self;

    /// Returns `true` if a backtrace is available.
    ///
    /// The default implementation returns `false`.
    fn has_backtrace(&self) -> bool {
        false
    }

    /// Formats the backtrace to the given formatter.
    ///
    /// This method writes directly to a [`Formatter`](fmt::Formatter) instead of
    /// returning `impl Display`, to avoid allocation and to keep this method
    /// object-safe (enabling `dyn ErrorSource` if other constraints are removed).
    ///
    /// The default implementation does nothing.
    fn fmt_backtrace(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Ok(())
    }
}

impl ErrorSource for AnyError {
    fn from_error<E: Error + 'static>(error: &E) -> Self {
        AnyError::new(error)
    }

    fn from_string(msg: impl ToString) -> Self {
        AnyError::error(msg)
    }

    /// Whether this error stores a backtrace.
    fn has_backtrace(&self) -> bool {
        self.backtrace().is_some()
    }

    /// Write the backtrace stored in this error, which was captured when the error was created.
    fn fmt_backtrace(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(bt) = self.backtrace() {
            write!(f, "{}", bt)
        } else {
            Ok(())
        }
    }
}

/// A wrapper that implements [`Display`](fmt::Display) for formatting backtraces.
pub struct BacktraceDisplay<'a, E: ErrorSource>(pub &'a E);

impl<E: ErrorSource> fmt::Display for BacktraceDisplay<'_, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.has_backtrace() {
            self.0.fmt_backtrace(f)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use anyerror::AnyError;

    use super::BacktraceDisplay;
    use super::ErrorSource;

    /// The backtrace is the one stored in the error, also when it came over the wire.
    #[cfg(feature = "serde")]
    #[test]
    fn test_any_error_reports_its_stored_backtrace() {
        let json = serde_json::json!({
            "typ": null, "msg": "read", "source": null, "context": [], "backtrace": "stored-marker"
        });
        let err: AnyError = serde_json::from_value(json).unwrap();

        assert!(err.has_backtrace());
        assert_eq!("stored-marker", BacktraceDisplay(&err).to_string());
    }

    /// An error without a stored backtrace reports none, also when backtrace capture is enabled.
    #[test]
    fn test_any_error_without_a_backtrace_reports_none() {
        let err = AnyError::error("read");
        assert_eq!(None, err.backtrace());

        assert!(!err.has_backtrace());
        assert_eq!("", BacktraceDisplay(&err).to_string());
    }
}
