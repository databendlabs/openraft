//! Configurable error source trait for wrapping arbitrary errors.

use std::error::Error;
use std::fmt::Display;

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

    /// Get the backtrace as a displayable value if captured.
    ///
    /// Returns `None` if backtrace is not available or not captured.
    /// The default implementation returns `None`.
    ///
    /// This method returns `impl Display` to avoid allocation when only
    /// displaying the backtrace. Call `.to_string()` if storage is needed.
    fn backtrace_display(&self) -> Option<impl Display + '_> {
        None::<&str>
    }
}

impl ErrorSource for AnyError {
    fn from_error<E: Error + 'static>(error: &E) -> Self {
        AnyError::new(error)
    }

    fn from_string(msg: impl ToString) -> Self {
        AnyError::error(msg)
    }

    fn backtrace_display(&self) -> Option<impl Display + '_> {
        anyerror::backtrace_str()
    }
}
