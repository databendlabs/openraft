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
/// The default implementation uses [`anyerror::AnyError`].
///
/// # Example
///
/// ```ignore
/// use openraft::error::ErrorSource;
///
/// #[derive(Debug, Clone, PartialEq, Eq)]
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
/// ```
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
