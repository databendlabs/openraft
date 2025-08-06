mod raft_term_impls;

use std::fmt::Debug;
use std::fmt::Display;

use openraft_macros::since;

use crate::base::OptionalFeatures;

/// Type representing a Raft term number.
///
/// A term is a logical clock in Raft that is used to detect obsolete information,
/// such as old leaders. It must be totally ordered and monotonically increasing.
///
/// Common implementations are provided for standard integer types like `u64`, `i64`, etc.
#[since(version = "0.10.0")]
pub trait RaftTerm
where Self: OptionalFeatures + Ord + Debug + Display + Copy + Default + 'static
{
    /// Returns the next term.
    ///
    /// Must satisfy: `self < self.next()`
    fn next(&self) -> Self;
}
