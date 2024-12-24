use std::fmt::Debug;
use std::fmt::Display;

use crate::log_id::RaftLogId;
use crate::LogId;
use crate::Membership;
use crate::OptionalSend;
use crate::OptionalSerde;
use crate::OptionalSync;
use crate::RaftTypeConfig;

/// Defines operations on an entry payload.
pub trait RaftPayload<C>
where C: RaftTypeConfig
{
    /// Return `true` if the entry payload is blank.
    fn is_blank(&self) -> bool;

    /// Return `Some(&Membership)` if the entry payload is a membership payload.
    fn get_membership(&self) -> Option<&Membership<C>>;
}

/// Defines operations on an entry.
pub trait RaftEntry<C>: RaftPayload<C> + RaftLogId<C>
where
    C: RaftTypeConfig,
    Self: OptionalSerde + Debug + Display + OptionalSend + OptionalSync,
{
    /// Create a new blank log entry.
    ///
    /// The returned instance must return `true` for `Self::is_blank()`.
    fn new_blank(log_id: LogId<C>) -> Self;

    /// Create a new membership log entry.
    ///
    /// The returned instance must return `Some()` for `Self::get_membership()`.
    fn new_membership(log_id: LogId<C>, m: Membership<C>) -> Self;
}

/// Build a raft log entry from app data.
///
/// A concrete Entry should implement this trait to let openraft create an entry when needed.
pub trait FromAppData<T> {
    /// Build a raft log entry from app data.
    fn from_app_data(t: T) -> Self;
}
