use std::fmt::Debug;
use std::fmt::Display;

use crate::base::OptionalFeatures;
use crate::log_id::ref_log_id::RefLogId;
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

    /// Return `Some(&Membership)` if the entry payload is a membership payload.
    fn get_membership(&self) -> Option<&Membership<C>>;
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

    /// Returns references to the components of this entry's log ID: the committed leader ID and
    /// index.
    ///
    /// The returned tuple contains:
    /// - A reference to the committed leader ID that proposed this log entry.
    /// - The index position of this entry in the log.
    ///
    /// Note: Although these components constitute a `LogId`, this method returns them separately
    /// rather than as a reference to `LogId`. This allows implementations to store these
    /// components directly without requiring a `LogId` field in their data structure.
    fn log_id_parts(&self) -> (&CommittedLeaderIdOf<C>, u64);

    fn set_log_id(&mut self, new: LogIdOf<C>);
}

pub trait RaftEntryExt<C>: RaftEntry<C>
where C: RaftTypeConfig
{
    /// Returns a lightweight [`RefLogId`] that contains the log id information.
    fn ref_log_id(&self) -> RefLogId<'_, C> {
        let (leader_id, index) = self.log_id_parts();
        RefLogId::new(leader_id, index)
    }

    fn to_log_id(&self) -> LogIdOf<C> {
        self.ref_log_id().to_log_id()
    }

    fn committed_leader_id(&self) -> &CommittedLeaderIdOf<C> {
        self.ref_log_id().committed_leader_id()
    }

    fn to_committed_leader_id(&self) -> CommittedLeaderIdOf<C> {
        self.committed_leader_id().clone()
    }

    fn index(&self) -> u64 {
        self.ref_log_id().index()
    }
}

impl<C, T> RaftEntryExt<C> for T
where
    C: RaftTypeConfig,
    T: RaftEntry<C>,
{
}
