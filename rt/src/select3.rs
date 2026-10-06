use openraft_macros::since;

/// The output of the future chosen by [`AsyncRuntime::select3`](crate::AsyncRuntime::select3).
#[since(version = "0.10.0")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Select3<A, B, C> {
    /// The first future completed.
    First(A),
    /// The second future completed.
    Second(B),
    /// The third future completed.
    Third(C),
}
