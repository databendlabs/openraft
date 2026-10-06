use openraft_macros::since;

/// The output of the future chosen by [`AsyncRuntime::select2`](crate::AsyncRuntime::select2).
#[since(version = "0.10.0")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Select2<A, B> {
    /// The first future completed.
    First(A),
    /// The second future completed.
    Second(B),
}
