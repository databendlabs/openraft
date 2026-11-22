use openraft_macros::since;

use crate::LogIndexOptionExt;
use crate::log_id::raft_log_id::RaftLogId;

/// This helper trait extracts information from an `Option<LogId>`.
#[since(
    version = "0.10.2",
    change = "`next_index()` panics on index `u64::MAX` in all builds"
)]
#[since(version = "0.10.0", change = "removed `C: RaftTypeConfig` generic parameter")]
pub trait LogIdOptionExt {
    /// Returns the log index if it is not a `None`.
    fn index(&self) -> Option<u64>;

    /// Returns the next log index.
    ///
    /// If self is `None`, it returns 0.
    ///
    /// # Panics
    ///
    /// Panics if the log index is `u64::MAX`, because its successor cannot be represented.
    fn next_index(&self) -> u64;
}

impl<T> LogIdOptionExt for Option<T>
where T: RaftLogId
{
    fn index(&self) -> Option<u64> {
        self.as_ref().map(|x| x.index())
    }

    fn next_index(&self) -> u64 {
        self.index().next_index()
    }
}

#[cfg(test)]
mod tests {
    use super::LogIdOptionExt;
    use crate::engine::testing::UTConfig;
    use crate::engine::testing::log_id;
    use crate::type_config::alias::LogIdOf;

    #[test]
    fn test_next_index_returns_representable_successors() {
        assert_eq!(0, None::<LogIdOf<UTConfig>>.next_index());
        assert_eq!(u64::MAX, Some(log_id(1, 1, u64::MAX - 1)).next_index());
    }

    /// The successor of the last index does not wrap to 0, the successor of `None`, in any build.
    #[test]
    #[should_panic(expected = "u64::MAX has no next log index")]
    fn test_next_index_panics_at_max() {
        Some(log_id(1, 1, u64::MAX)).next_index();
    }
}
