//! Provides a marker trait for provided trait methods that implementations should inherit.

/// Marks a provided trait method whose default implementation an implementor should inherit.
///
/// A trait method declared with `where Self: Final` has a default that implementations are
/// expected to keep. `Final` is implemented for every sized type, and its module is private to the
/// crate, so code outside Openraft cannot name it.
///
/// The marker does not prevent an override: an implementation of a trait method may leave out the
/// method's `where` clauses, so a downstream implementation can define the method without naming
/// `Final`. A trait that uses the marker therefore documents what an override must preserve, as
/// [`RaftEntry`](crate::entry::RaftEntry) does.
pub trait Final {}

impl<T> Final for T {}
