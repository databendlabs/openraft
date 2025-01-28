use std::fmt::Debug;
use std::fmt::Display;

use openraft_macros::since;

use crate::base::finalized::Final;
use crate::base::OptionalFeatures;
use crate::log_id::RaftLogId;
use crate::type_config::alias::LogIdOf;
use crate::Membership;
use crate::RaftTypeConfig;

/// Defines operations on an entry payload.
pub trait RaftPayload<C>
where C: RaftTypeConfig
{
    /// Return `Some(Membership)` if the entry payload is a membership payload.
    fn get_membership(&self) -> Option<Membership<C>>;
}

/// Defines operations on an entry.
pub trait RaftEntry<C>
where
    C: RaftTypeConfig,
    Self: OptionalFeatures + Debug + Display,
    Self: RaftPayload<C> + RaftLogId<C>,
{
    /// Create a new blank log entry.
    ///
    /// The returned instance must return `true` for `Self::is_blank()`.
    fn new_blank(log_id: LogIdOf<C>) -> Self;

    /// Create a new membership log entry.
    ///
    /// The returned instance must return `Some()` for `Self::get_membership()`.
    fn new_membership(log_id: LogIdOf<C>, m: Membership<C>) -> Self;

    /// Returns the `LogId` of this entry.
    #[since(version = "0.10.0")]
    fn log_id(&self) -> LogIdOf<C>
    where Self: Final {
        self.get_log_id().clone()
    }

    /// Returns the index of this log entry.
    #[since(version = "0.10.0")]
    fn index(&self) -> u64
    where Self: Final {
        self.get_log_id().index()
    }
}

/// Build a raft log entry from app data.
///
/// A concrete Entry should implement this trait to let openraft create an entry when needed.
pub trait FromAppData<T> {
    /// Build a raft log entry from app data.
    fn from_app_data(t: T) -> Self;
}
