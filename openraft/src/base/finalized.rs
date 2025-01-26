//! Provides a marker trait to prevent external implementation of trait methods.

/// A marker trait used to prevent specific already auto-implemented trait methods from being
/// re-implemented outside their defining crate.
///
/// This is achieved by adding this non-referencable marker trait to a trait method signature.
///
/// # Example
///
/// The following code demonstrates how `Final` prevents external implementation:
///
/// ```ignore
/// pub trait Trait {
///     // This method cannot be implemented by users because it requires
///     // the private `Final` trait in its bounds
///     fn unimplementable(&self) where Self: Final {
///         self.user_impl_this();
///     }
///
///     // This method can be implemented by users
///     fn user_impl_this(&self);
/// }
///
/// pub struct MyType;
///
/// impl Trait for MyType {
///     // Attempting to implement this method will fail to compile
///     // because `Final` is not accessible from outside the crate
///     fn unimplementable(&self) where Self: Final {}
///
///     fn user_impl_this(&self) {
///         println!("This implementation is allowed");
///     }
/// }
/// ```
pub trait Final {}

impl<T> Final for T {}
