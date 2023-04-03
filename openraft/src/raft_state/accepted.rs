use crate::LeaderId;
use crate::LogId;
use crate::NodeId;

/// A range of log entries that is accepted by a follower.
///
/// It is similar to the `accepted` value in paxos.
/// But it is not persisted and is not considered when election.
///
/// It is only used when determining the last committable log id.
///
/// The accepted log id must be present. When the follower truncates its log, `accepted` should be
/// reset.
#[derive(Debug, Clone)]
#[derive(Default)]
#[derive(PartialEq, Eq)]
pub(crate) struct Accepted<NID: NodeId> {
    /// From which leader this range of log is accepted.
    leader_id: LeaderId<NID>,

    /// The last log id that is accepted.
    log_id: Option<LogId<NID>>,
}

impl<NID: NodeId> Accepted<NID> {
    /// Create a new `Accepted` with the given leader id and log id.
    pub(crate) fn new(leader_id: LeaderId<NID>, log_id: Option<LogId<NID>>) -> Self {
        Self { leader_id, log_id }
    }

    pub(crate) fn leader_id(&self) -> &LeaderId<NID> {
        &self.leader_id
    }

    /// Get the last accepted log id from the given leader.
    ///
    /// If the given leader is not the leader of this `Accepted`, return `None`.
    pub(crate) fn last_accepted_log_id(&self, leader_id: &LeaderId<NID>) -> Option<&LogId<NID>> {
        if leader_id == &self.leader_id {
            self.log_id.as_ref()
        } else {
            None
        }
    }
}
