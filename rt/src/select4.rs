use openraft_macros::since;

/// The output of the future chosen by [`AsyncRuntime::select4`](crate::AsyncRuntime::select4).
#[since(version = "0.10.0")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Select4<A, B, C, D> {
    /// The first future completed.
    First(A),
    /// The second future completed.
    Second(B),
    /// The third future completed.
    Third(C),
    /// The fourth future completed.
    Fourth(D),
}
