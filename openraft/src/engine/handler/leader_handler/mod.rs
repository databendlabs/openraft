use crate::RaftState;
use crate::RaftTypeConfig;
use crate::engine::Command;
use crate::engine::EngineConfig;
use crate::engine::EngineOutput;
use crate::engine::handler::replication_handler::ReplicationHandler;
use crate::engine::leader_log_ids::LeaderLogIds;
use crate::engine::quorum_loss_probe::QuorumLossProbe;
use crate::entry::RaftEntry;
use crate::proposer::Leader;
use crate::proposer::WriteQuorumSet;
use crate::raft::linearizable_read::Linearizer;
use crate::raft::message::TransferLeaderRequest;
use crate::raft_state::IOId;
use crate::raft_state::LogStateReader;
use crate::replication::ReplicationSessionId;
use crate::storage::RaftStateMachine;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::BatchOf;
use crate::type_config::alias::CommittedLeaderIdOf;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::PayloadOf;

#[cfg(test)]
mod append_entries_test;
#[cfg(test)]
mod get_linearizer_test;
#[cfg(test)]
mod send_heartbeat_test;
#[cfg(test)]
mod transfer_leader_test;

/// Handle leader operations.
///
/// - Append new logs;
/// - Change membership;
/// - etc.
pub(crate) struct LeaderHandler<'x, C, SM = ()>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    pub(crate) config: &'x mut EngineConfig<C>,
    pub(crate) leader: &'x mut Leader<C, WriteQuorumSet<C>>,
    pub(crate) state: &'x mut RaftState<C>,
    pub(crate) output: &'x mut EngineOutput<C, SM>,
}

impl<'x, C, SM> LeaderHandler<'x, C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    pub(crate) fn new(
        config: &'x mut EngineConfig<C>,
        leader: &'x mut Leader<C, WriteQuorumSet<C>>,
        state: &'x mut RaftState<C>,
        output: &'x mut EngineOutput<C, SM>,
    ) -> Self {
        Self {
            config,
            leader,
            state,
            output,
        }
    }

    /// Return whether the leader's lease is valid.
    pub(crate) fn is_lease_valid(&self) -> bool {
        self.leader.is_lease_valid(self.config.timer_config.leader_lease)
    }

    /// Append new log entries by a leader.
    ///
    /// A membership entry in `payloads` becomes the effective membership at once, and the entries
    /// from it on commit by its write quorums alone. This method checks none of the conditions that
    /// make that safe. A caller that appends a membership entry must ensure that:
    ///
    /// - `payloads` holds at most one membership entry.
    /// - The last membership log is committed.
    /// - The proposed membership is valid, and a direct step from that committed membership to it
    ///   is allowed: it is their joint membership, or a membership that
    ///   [`RaftMembership::supports_direct_transition_to()`] accepts. See
    ///   [`MembershipState::next_membership()`] and
    ///   [`MembershipState::validate_append_membership()`].
    /// - The blank log this leader appended when it was established is committed. A committed log
    ///   of another leader, even one of the same term, does not count. See
    ///   `RaftCore::ensure_leader_log_committed()`.
    ///
    /// `RaftCore::change_membership()` and `RaftCore::append_membership()` check them all. Entries
    /// without membership need none of them, such as the blank log of a new leader.
    ///
    /// [`RaftMembership::supports_direct_transition_to()`]: crate::membership::RaftMembership::supports_direct_transition_to
    /// [`MembershipState::next_membership()`]: crate::raft_state::MembershipState::next_membership
    /// [`MembershipState::validate_append_membership()`]: crate::raft_state::MembershipState::validate_append_membership
    #[tracing::instrument(level = "debug", skip(self, payloads))]
    pub(crate) fn leader_append_entries<I>(&mut self, payloads: I) -> Option<LeaderLogIds<CommittedLeaderIdOf<C>>>
    where I: IntoIterator<Item = PayloadOf<C>> + AsRef<[PayloadOf<C>]> {
        let log_ids = self.leader.assign_log_ids(payloads.as_ref().len())?;

        self.state.extend_log_ids_from_same_leader(log_ids.clone());

        let mut membership_entry = None;
        let entries: BatchOf<C, _> = payloads
            .into_iter()
            .zip(log_ids.clone())
            .map(|(payload, log_id)| {
                tracing::debug!("assign log id: {}", log_id);
                let entry = C::Entry::new(log_id, payload);
                if let Some(m) = entry.get_membership() {
                    debug_assert!(
                        membership_entry.is_none(),
                        "only one membership entry is allowed in a batch"
                    );
                    membership_entry = Some((entry.log_id(), m));
                }
                entry
            })
            .collect();

        self.state.accept_log_io(IOId::new_log_io(
            self.leader.committed_vote.clone(),
            self.leader.last_log_id().cloned(),
        ));

        self.output.push_command(Command::AppendEntries {
            // A leader should always use the leader's vote.
            // It is allowed to be different from local vote.
            committed_vote: self.leader.committed_vote.clone(),
            entries,
        });

        let mut rh = self.replication_handler();

        // From this entry on, entries commit by the write quorums of the new membership alone. The
        // caller checked the step from the committed membership: each read quorum of either side
        // intersects each write quorum of the other. So a leader that either side elects holds
        // every entry that the other side committed.
        if let Some((log_id, m)) = membership_entry {
            rh.append_membership(&log_id, &m);
        }

        // Proposing entries is what starts their replication. With a membership entry, the
        // replication streams were rebuilt for the new membership just above.
        rh.initiate_replication();

        Some(log_ids)
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn send_heartbeat(&mut self, bypass_min_interval: bool) -> bool {
        if !self.heartbeat_is_allowed(C::now()) {
            return false;
        }

        let session_id = self.replication_session_id();

        self.output.push_command(Command::BroadcastHeartbeat {
            session_id,
            bypass_min_interval,
        });
        true
    }

    fn heartbeat_is_allowed(&self, now: InstantOf<C>) -> bool {
        self.quorum_loss_probe().is_none_or(|probe| probe.quiet_for(now).is_none())
    }

    /// Return the quiet periods this Leader keeps after losing its quorum, or `None` if it keeps
    /// none.
    pub(crate) fn quorum_loss_probe(&self) -> Option<QuorumLossProbe<InstantOf<C>>> {
        let interval = self.config.quorum_loss_probe_interval?;
        if self.leader.is_self_quorum() {
            return None;
        }

        let quorum_acked_at = self.leader.last_quorum_acked_time();
        let quorum_acked_at = quorum_acked_at.or(self.state.vote_last_modified())?;
        Some(QuorumLossProbe {
            quorum_acked_at,
            leader_lease: self.config.timer_config.leader_lease,
            interval,
        })
    }

    /// Capture the log IDs for a linearizable read.
    ///
    /// See: [Read Operation](crate::docs::protocol::read)
    pub(crate) fn get_linearizer(&self) -> Linearizer<C> {
        Linearizer::new(
            self.config.id.clone(),
            self.leader.noop_log_id.clone(),
            self.state.local_committed().cloned(),
            self.state.io_applied().cloned(),
        )
    }

    /// Disable proposing new logs for this Leader and transfer Leader to another node
    ///
    /// If `to` has not taken over within an election timeout, this Leader steps down and starts an
    /// election. Otherwise a transfer to a learner, or to a node that is down, would block the
    /// cluster forever when no other node can win an election, such as in a cluster with one
    /// voter.
    pub(crate) fn transfer_leader(&mut self, to: C::NodeId) {
        let timeout = self.config.timer_config.election_timeout;
        self.leader.mark_transfer(to.clone(), timeout);
        self.state.vote.disable_lease();

        self.output.push_command(Command::BroadcastTransferLeader {
            req: TransferLeaderRequest::new(
                self.leader.committed_vote.clone().into_vote(),
                to,
                self.leader.last_log_id().cloned(),
            ),
        });
    }

    /// Return the ID of this leader's replication session: its committed vote and the log ID of
    /// the effective membership.
    pub(crate) fn replication_session_id(&self) -> ReplicationSessionId<C> {
        let committed_vote = self.leader.committed_vote.clone();
        let membership_log_id = self.state.membership_state.effective().log_id();
        ReplicationSessionId::new(committed_vote, membership_log_id.clone())
    }

    pub(crate) fn replication_handler(&mut self) -> ReplicationHandler<'_, C, SM> {
        ReplicationHandler::new(self.config, self.leader, self.state, self.output)
    }
}
