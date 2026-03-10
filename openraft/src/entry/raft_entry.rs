use std::fmt::Debug;
use std::fmt::Display;

use openraft_macros::since;

use crate::AppData;
use crate::EntryPayload;
use crate::Membership;
use crate::base::OptionalFeatures;
use crate::base::finalized::Final;
use crate::entry::RaftPayload;
use crate::log_id::LogId;
use crate::node::Node;
use crate::node::NodeId;
use crate::vote::leader_id::raft_committed_leader_id::RaftCommittedLeaderId;

/// Defines operations on an entry.
#[since(
    version = "0.10.0",
    change = "removed `C: RaftTypeConfig` generic parameter, added associated types"
)]
pub trait RaftEntry
where
    Self: OptionalFeatures + Debug + Display,
    Self: RaftPayload<Self::NodeId, Self::Node>,
{
    /// The committed leader ID type used in log IDs.
    #[since(version = "0.10.0")]
    type CommittedLeaderId: RaftCommittedLeaderId;

    /// Application-specific data type stored in log entries.
    #[since(version = "0.10.0")]
    type D: AppData;

    /// The node ID type.
    #[since(version = "0.10.0")]
    type NodeId: NodeId;

    /// The node type.
    #[since(version = "0.10.0")]
    type Node: Node;

    /// Create a new log entry with log id and payload of application data or membership config.
    #[since(version = "0.10.0")]
    fn new(log_id: LogId<Self::CommittedLeaderId>, payload: EntryPayload<Self::D, Self::NodeId, Self::Node>) -> Self;

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

    /// Create a new blank log entry.
    #[since(version = "0.10.0", change = "become a default method")]
    fn new_blank(log_id: LogId<Self::CommittedLeaderId>) -> Self
    where Self: Final + Sized {
        Self::new(log_id, EntryPayload::Blank)
    }

    /// Create a new normal log entry that contains application data.
    #[since(version = "0.10.0", change = "become a default method")]
    fn new_normal(log_id: LogId<Self::CommittedLeaderId>, data: Self::D) -> Self
    where Self: Final + Sized {
        Self::new(log_id, EntryPayload::Normal(data))
    }

    /// Create a new membership log entry.
    ///
    /// The returned instance must return `Some()` for `Self::get_membership()`.
    #[since(version = "0.10.0", change = "become a default method")]
    fn new_membership(log_id: LogId<Self::CommittedLeaderId>, m: Membership<Self::NodeId, Self::Node>) -> Self
    where Self: Final + Sized {
        Self::new(log_id, EntryPayload::Membership(m))
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
