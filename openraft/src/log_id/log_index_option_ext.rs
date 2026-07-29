use openraft_macros::since;

/// This helper trait extracts information from an `Option<LogIndex>`.
///
/// In openraft, `LogIndex` is a `u64`.
#[since(version = "0.10.0", change = "removed unused method `add()`")]
pub trait LogIndexOptionExt {
    /// Return the next log index.
    ///
    /// If self is `None`, it returns 0.
    fn next_index(&self) -> u64;

    /// Return the previous log index.
    ///
    /// If self is `None`, it panics.
    fn prev_index(&self) -> Self;
}

impl LogIndexOptionExt for Option<u64> {
    fn next_index(&self) -> u64 {
        match self {
            None => 0,
            Some(v) => v + 1,
        }
    }

    fn prev_index(&self) -> Self {
        match self {
            None => {
                panic!("None has no previous value");
            }
            Some(v) => {
                if *v == 0 {
                    None
                } else {
                    Some(*v - 1)
                }
            }
        }
    }
}
