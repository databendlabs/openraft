use std::fmt::Debug;
use std::fmt::Display;

use crate::base::OptionalFeatures;
use crate::log_id::ref_log_id::RefLogId;
use crate::log_id::RaftLogId;
use crate::type_config::alias::CommittedLeaderIdOf;
use crate::type_config::alias::LogIdOf;
use crate::Membership;
use crate::RaftTypeConfig;

/// Defines operations on an entry payload.
pub trait RaftPayload<C>
where C: RaftTypeConfig
{
    /// Return `true` if the entry payload is blank.
    fn is_blank(&self) -> bool;

    /// Return `Some(Membership)` if the entry payload is a membership payload.
    fn get_membership(&self) -> Option<Membership<C>>;
}

/// Defines operations on an entry.
pub trait RaftEntry<C>
where
    C: RaftTypeConfig,
    Self: OptionalFeatures + Debug + Display,
    Self: RaftPayload<C>,
{
    /// Create a new blank log entry.
    ///
    /// The returned instance must return `true` for `Self::is_blank()`.
    fn new_blank(log_id: LogIdOf<C>) -> Self;

    /// Create a new normal log entry that contains application data.
    fn new_normal(log_id: LogIdOf<C>, data: C::D) -> Self;

    /// Create a new membership log entry.
    ///
    /// The returned instance must return `Some()` for `Self::get_membership()`.
    fn new_membership(log_id: LogIdOf<C>, m: Membership<C>) -> Self;

    /// Creates a lightweight [`RefLogId`] that references the log id information.
    fn ref_log_id(&self) -> RefLogId<'_, C>;

    fn set_log_id(&mut self, new: &LogIdOf<C>);
}

pub trait RaftEntryExt<C>: RaftEntry<C>
where C: RaftTypeConfig
{
    fn to_log_id(&self) -> LogIdOf<C> {
        self.ref_log_id().to_log_id()
    }

    fn committed_leader_id(&self) -> &CommittedLeaderIdOf<C> {
        AsRef::<LogIdOf<C>>::as_ref(self).leader_id()
    }

    fn to_committed_leader_id(&self) -> CommittedLeaderIdOf<C> {
        self.committed_leader_id().clone()
    }

    fn index(&self) -> u64 {
        AsRef::<LogIdOf<C>>::as_ref(self).index()
    }
}

impl<C, T> RaftEntryExt<C> for T
where
    C: RaftTypeConfig,
    T: RaftEntry<C>,
{
}
