use std::time::Duration;

use crate::RaftTypeConfig;
use crate::raft::linearizable_read::Linearizer;
use crate::raft::linearizable_read::LinearizerOption;
use crate::type_config::alias::CommittedVoteOf;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::LogIdOf;

/// A coherent leader snapshot used to authorize cached linearizable reads.
///
/// The core must publish committed progress before dispatching apply work and clear the snapshot
/// before relinquishing leadership. The quorum clock records acknowledged RPC sending time.
/// Applied progress is an independent hint and is read from metrics by the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LeaseReadState<C>
where C: RaftTypeConfig
{
    pub(crate) leader_vote: CommittedVoteOf<C>,
    pub(crate) noop_log_id: LogIdOf<C>,
    pub(crate) committed: Option<LogIdOf<C>>,
    pub(crate) quorum_acked_at: Option<InstantOf<C>>,
    pub(crate) leader_lease: Duration,
    pub(crate) self_quorum: bool,
}

impl<C> LeaseReadState<C>
where C: RaftTypeConfig
{
    /// Returns a linearizer only when the request permits a fresh cached acknowledgement.
    pub(crate) fn try_linearizer(
        &self,
        node_id: C::NodeId,
        option: &LinearizerOption,
        now: InstantOf<C>,
    ) -> Option<Linearizer<C>> {
        #[cfg(debug_assertions)]
        {
            let leader_id = self.leader_vote.committed_leader_id();
            let noop_leader_id = self.noop_log_id.committed_leader_id();
            debug_assert_eq!(noop_leader_id, &leader_id, "snapshot no-op must belong to leader vote");
        }
        let allows_cached_read = self.allows_cached_read(option, now);
        if !allows_cached_read {
            return None;
        }
        let linearizer = Linearizer::new(node_id, self.noop_log_id.clone(), self.committed.clone(), None);
        Some(linearizer)
    }

    fn allows_cached_read(&self, option: &LinearizerOption, now: InstantOf<C>) -> bool {
        let max_age = option.effective_max_quorum_ack_age(self.leader_lease);
        if max_age.is_zero() {
            return false;
        }
        if self.self_quorum {
            return true;
        }
        let Some(acked_at) = self.quorum_acked_at else {
            return false;
        };
        let expires_at = acked_at + max_age;
        now < expires_at
    }
}

#[cfg(test)]
mod lease_read_state_test;
