use std::fmt::Debug;
use std::fmt::Display;

use crate::log_id::RaftLogId;
use crate::LogId;
use crate::Membership;
use crate::Node;
use crate::NodeId;
use crate::OptionalSend;
use crate::OptionalSerde;
use crate::OptionalSync;

/// Defines operations on an entry payload.
pub trait RaftPayload<NID, N>
where
    N: Node,
    NID: NodeId,
{
    /// Return `true` if the entry payload is blank.
    fn is_blank(&self) -> bool;

    /// Return `Some(&Membership)` if the entry payload is a membership payload.
    fn get_membership(&self) -> Option<&Membership<NID, N>>;
}

/// Defines operations on an entry.
pub trait RaftEntry<NID, N>: RaftPayload<NID, N> + RaftLogId<NID>
where
    N: Node,
    NID: NodeId,
    Self: OptionalSerde + Debug + Display + OptionalSend + OptionalSync,
{
    /// Create a new blank log entry.
    ///
    /// The returned instance must return `true` for `Self::is_blank()`.
    fn new_blank(log_id: LogId<NID>) -> Self;

    /// Create a new membership log entry.
    ///
    /// The returned instance must return `Some()` for `Self::get_membership()`.
    fn new_membership(log_id: LogId<NID>, m: Membership<NID, N>) -> Self;
}

/// Build a raft log entry from app data.
///
/// A concrete Entry should implement this trait to let openraft create an entry when needed.
pub trait FromAppData<T> {
    /// Build a raft log entry from app data.
    fn from_app_data(t: T) -> Self;
}
