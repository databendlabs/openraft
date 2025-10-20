use std::fmt::Debug;
use std::fmt::Display;

use openraft_macros::since;

use crate::EntryPayload;
use crate::Membership;
use crate::RaftTypeConfig;
use crate::base::OptionalFeatures;
use crate::base::finalized::Final;
use crate::entry::RaftPayload;
use crate::type_config::alias::CommittedLeaderIdOf;
use crate::type_config::alias::LogIdOf;

/// Defines operations on an entry.
pub trait RaftEntry<C>
where
    C: RaftTypeConfig,
    Self: OptionalFeatures + Debug + Display,
    Self: RaftPayload<C>,
{
    /// Create a new log entry with log id and payload of application data or membership config.
    #[since(version = "0.10.0")]
    fn new(log_id: LogIdOf<C>, payload: EntryPayload<C>) -> Self;

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
    #[since(version = "0.10.0")]
    fn log_id_parts(&self) -> (&CommittedLeaderIdOf<C>, u64);

    /// Set the log ID of this entry.
    #[since(version = "0.10.0", change = "use owned argument log id")]
    fn set_log_id(&mut self, new: LogIdOf<C>);

    /// Create a new blank log entry.
    #[since(version = "0.10.0", change = "become a default method")]
    fn new_blank(log_id: LogIdOf<C>) -> Self
    where Self: Final + Sized {
        Self::new(log_id, EntryPayload::Blank)
    }

    /// Create a new normal log entry that contains application data.
    #[since(version = "0.10.0", change = "become a default method")]
    fn new_normal(log_id: LogIdOf<C>, data: C::D) -> Self
    where Self: Final + Sized {
        Self::new(log_id, EntryPayload::Normal(data))
    }

    /// Create a new membership log entry.
    ///
    /// The returned instance must return `Some()` for `Self::get_membership()`.
    #[since(version = "0.10.0", change = "become a default method")]
    fn new_membership(log_id: LogIdOf<C>, m: Membership<C>) -> Self
    where Self: Final + Sized {
        Self::new(log_id, EntryPayload::Membership(m))
    }

    /// Returns the `LogId` of this entry.
    #[since(version = "0.10.0")]
    fn log_id(&self) -> LogIdOf<C>
    where Self: Final {
        let (leader_id, index) = self.log_id_parts();
        LogIdOf::<C>::new(leader_id.clone(), index)
    }

    /// Returns the index of this log entry.
    #[since(version = "0.10.0")]
    fn index(&self) -> u64
    where Self: Final {
        self.log_id_parts().1
    }
}
