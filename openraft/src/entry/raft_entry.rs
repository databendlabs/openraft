use std::fmt::Debug;
use std::fmt::Display;

use openraft_macros::since;

use crate::Membership;
use crate::base::OptionalFeatures;
use crate::base::finalized::Final;
use crate::entry::RaftPayload;
use crate::log_id::LogId;
use crate::vote::RaftCommittedLeaderId;

/// Defines operations on an entry.
#[since(
    version = "0.10.0",
    change = "replaced `D`, `NodeId`, and `Node` associated types with `Payload`"
)]
#[since(
    version = "0.10.0",
    change = "moved membership inspection from `RaftPayload` to `RaftEntry`"
)]
#[since(
    version = "0.10.0",
    change = "removed `C: RaftTypeConfig` generic parameter, added associated types"
)]
pub trait RaftEntry
where Self: OptionalFeatures + Debug + Display
{
    /// The committed leader ID type used in log IDs.
    #[since(version = "0.10.0")]
    type CommittedLeaderId: RaftCommittedLeaderId;

    /// The payload stored in log entries.
    #[since(version = "0.10.0")]
    type Payload: RaftPayload;

    /// Create a new log entry with a log ID and configured payload.
    #[since(version = "0.10.0", change = "accept configured payload type")]
    #[since(version = "0.10.0")]
    fn new(log_id: LogId<Self::CommittedLeaderId>, payload: Self::Payload) -> Self;

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
    fn log_id_parts(&self) -> (&Self::CommittedLeaderId, u64);

    /// Set the log ID of this entry.
    #[since(version = "0.10.0", change = "use owned argument log id")]
    fn set_log_id(&mut self, new: LogId<Self::CommittedLeaderId>);

    /// Return `Some(Membership)` if this entry contains a membership payload.
    #[since(version = "0.10.0", change = "derive membership type from `Payload`")]
    #[since(version = "0.10.0")]
    fn get_membership(
        &self,
    ) -> Option<Membership<<Self::Payload as RaftPayload>::NodeId, <Self::Payload as RaftPayload>::Node>>;

    /// Create a new blank log entry.
    #[since(version = "0.10.0", change = "construct configured payload type")]
    #[since(version = "0.10.0", change = "become a default method")]
    fn new_blank(log_id: LogId<Self::CommittedLeaderId>) -> Self
    where Self: Final + Sized {
        Self::new(log_id, Self::Payload::blank())
    }

    /// Create a new normal log entry that contains application data.
    #[since(version = "0.10.0", change = "construct configured payload type")]
    #[since(version = "0.10.0", change = "become a default method")]
    fn new_normal(log_id: LogId<Self::CommittedLeaderId>, data: <Self::Payload as RaftPayload>::D) -> Self
    where Self: Final + Sized {
        Self::new(log_id, Self::Payload::normal(data))
    }

    /// Create a new membership log entry.
    ///
    /// The returned instance must return `Some()` for `Self::get_membership()`.
    #[since(version = "0.10.0", change = "construct configured payload type")]
    #[since(version = "0.10.0", change = "become a default method")]
    fn new_membership(
        log_id: LogId<Self::CommittedLeaderId>,
        m: Membership<<Self::Payload as RaftPayload>::NodeId, <Self::Payload as RaftPayload>::Node>,
    ) -> Self
    where
        Self: Final + Sized,
    {
        Self::new(log_id, Self::Payload::membership(m))
    }

    /// Returns the `LogId` of this entry.
    #[since(version = "0.10.0")]
    fn log_id(&self) -> LogId<Self::CommittedLeaderId>
    where Self: Final {
        let (leader_id, index) = self.log_id_parts();
        LogId::new(leader_id.clone(), index)
    }

    /// Returns the index of this log entry.
    #[since(version = "0.10.0")]
    fn index(&self) -> u64
    where Self: Final {
        self.log_id_parts().1
    }
}
