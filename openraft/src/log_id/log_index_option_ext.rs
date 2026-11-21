use openraft_macros::since;

/// This helper trait extracts information from an `Option<LogIndex>`.
///
/// In openraft, `LogIndex` is a `u64`.
#[since(version = "0.10.2", change = "`next_index()` panics on `u64::MAX` in all builds")]
#[since(version = "0.10.0", change = "removed unused method `add()`")]
pub trait LogIndexOptionExt {
    /// Return the next log index.
    ///
    /// If self is `None`, it returns 0.
    ///
    /// # Panics
    ///
    /// Panics if self is `Some(u64::MAX)` because the successor cannot be represented.
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
            Some(v) => v.checked_add(1).expect("u64::MAX has no next log index"),
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

#[cfg(test)]
mod tests {
    use super::LogIndexOptionExt;

    #[test]
    fn next_index_returns_representable_successors() {
        assert_eq!(None.next_index(), 0);
        assert_eq!(Some(u64::MAX - 1).next_index(), u64::MAX);
    }

    #[test]
    #[should_panic(expected = "u64::MAX has no next log index")]
    fn next_index_panics_at_max() {
        Some(u64::MAX).next_index();
    }
}
