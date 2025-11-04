use std::time::Duration;

use validit::Valid;

use crate::LogIdOptionExt;
use crate::Membership;
use crate::RaftTypeConfig;
use crate::core::ServerState;
use crate::core::raft_msg::AppendEntriesTx;
use crate::core::sm;
use crate::display_ext::DisplayOptionExt;
use crate::display_ext::DisplaySliceExt;
use crate::engine::Command;
use crate::engine::Condition;
use crate::engine::EngineOutput;
use crate::engine::Respond;
use crate::engine::engine_config::EngineConfig;
use crate::engine::handler::establish_handler::EstablishHandler;
use crate::engine::handler::following_handler::FollowingHandler;
use crate::engine::handler::leader_handler::LeaderHandler;
use crate::engine::handler::log_handler::LogHandler;
use crate::engine::handler::replication_handler::ReplicationHandler;
use crate::engine::handler::server_state_handler::ServerStateHandler;
use crate::engine::handler::snapshot_handler::SnapshotHandler;
use crate::engine::handler::vote_handler::VoteHandler;
use crate::entry::RaftEntry;
use crate::entry::RaftPayload;
use crate::error::ForwardToLeader;
use crate::error::InitializeError;
use crate::error::NotAllowed;
use crate::error::NotInMembers;
use crate::error::RejectAppendEntries;
use crate::proposer::Candidate;
use crate::proposer::Leader;
use crate::proposer::LeaderQuorumSet;
use crate::proposer::LeaderState;
use crate::proposer::leader_state::CandidateState;
use crate::raft::AppendEntriesResponse;
use crate::raft::SnapshotResponse;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::raft::message::ClientWriteResult;
use crate::raft::responder::Responder;
use crate::raft_state::IOId;
use crate::raft_state::LogStateReader;
use crate::raft_state::RaftState;
use crate::storage::Snapshot;
use crate::storage::SnapshotMeta;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::LeaderIdOf;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::OneshotSenderOf;
use crate::type_config::alias::SnapshotDataOf;
use crate::type_config::alias::VoteOf;
use crate::type_config::alias::WriteResponderOf;
use crate::vote::RaftLeaderId;
use crate::vote::RaftTerm;
use crate::vote::RaftVote;
use crate::vote::raft_vote::RaftVoteExt;

/// Raft protocol algorithm.
///
/// It implement the complete raft algorithm except does not actually update any states.
/// But instead, it output commands to let a `RaftRuntime` implementation execute them to actually
/// update the states such as append-log or save-vote by execute .
///
/// This structure only contains necessary information to run raft algorithm,
/// but none of the application specific data.
/// TODO: make the fields private
#[derive(Debug)]
pub(crate) struct Engine<C>
where C: RaftTypeConfig
{
    pub(crate) config: EngineConfig<C>,

    /// The state of this raft node.
    pub(crate) state: Valid<RaftState<C>>,

    // TODO: add a Voting state as a container.
    /// Whether a greater log id is seen during election.
    ///
    /// If it is true, then this node **may** not become a leader therefore the election timeout
    /// should be greater.
    pub(crate) seen_greater_log: bool,

    /// Represents the Leader state.
    pub(crate) leader: LeaderState<C>,

    /// Represents the Candidate state within Openraft.
    ///
    /// A Candidate can coexist with a Leader in the system.
    /// This scenario is typically used to transition the Leader to a higher term (vote)
    /// without losing leadership status.
    pub(crate) candidate: CandidateState<C>,

    /// Output entry for the runtime.
    pub(crate) output: EngineOutput<C>,
}

impl<C> Engine<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(init_state: RaftState<C>, config: EngineConfig<C>) -> Self {
        Self {
            config,
            state: Valid::new(init_state),
            seen_greater_log: false,
            leader: None,
            candidate: None,
            output: EngineOutput::new(4096),
        }
    }

    /// Create a new candidate state and return the mutable reference to it.
    ///
    /// The candidate `last_log_id` is initialized with the attributes of Acceptor part:
    /// [`RaftState`]
    pub(crate) fn new_candidate(&mut self, vote: VoteOf<C>) -> &mut Candidate<C, LeaderQuorumSet<C>> {
        let now = C::now();
        let last_log_id = self.state.last_log_id().cloned();

        let membership = self.state.membership_state.effective().membership();

        self.candidate = Some(Candidate::new(
            now,
            vote,
            last_log_id,
            membership.to_quorum_set(),
            membership.learner_ids(),
        ));

        self.candidate.as_mut().unwrap()
    }

    /// Create a default Engine for testing.
    #[allow(dead_code)]
    pub(crate) fn testing_default(id: C::NodeId) -> Self {
        let config = EngineConfig::new_default(id);
        let state = RaftState::default();
        Self::new(state, config)
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn startup(&mut self) {
        // Allows starting up as a leader.

        tracing::info!(
            "startup begin: state: {:?}, is_leader: {}, is_voter: {}",
            self.state,
            self.state.is_leader(&self.config.id),
            self.state.membership_state.effective().is_voter(&self.config.id)
        );

        // TODO: replace all the following codes with one update_internal_server_state;
        // Previously it is a leader. restore it as leader at once
        if self.state.is_leader(&self.config.id) {
            self.vote_handler().update_internal_server_state();
            return;
        }

        let server_state = if self.state.membership_state.effective().is_voter(&self.config.id) {
            ServerState::Follower
        } else {
            ServerState::Learner
        };

        self.state.server_state = server_state;

        tracing::info!(
            "startup done: id={} target_state: {:?}",
            self.config.id,
            self.state.server_state
        );
    }

    /// Initialize a node by appending the first log.
    ///
    /// - The first log has to be membership config log.
    /// - The node has to contain no logs at all and the vote is the minimal value. See: [Conditions
    ///   for initialization][precondition].
    ///
    ///
    /// Appending the very first log is slightly different from appending log by a leader or
    /// follower. This step is not confined by the consensus protocol and has to be dealt with
    /// differently.
    ///
    /// [precondition]: crate::docs::cluster_control::cluster_formation#preconditions-for-initialization
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn initialize(&mut self, mut entry: C::Entry) -> Result<(), InitializeError<C>> {
        self.check_initialize()?;

        // The very first log id
        entry.set_log_id(LogIdOf::<C>::default());

        let m = entry.get_membership().expect("the only log entry for initializing has to be membership log");
        self.check_members_contain_me(&m)?;

        // FollowingHandler requires vote to be committed.
        let vote = <VoteOf<C> as RaftVote<C>>::from_leader_id(Default::default(), true);
        self.state.vote.update(C::now(), Duration::default(), vote);
        self.following_handler().do_append_entries(vec![entry]);

        // With the new config, start to elect to become leader
        self.elect();

        Ok(())
    }

    /// Start to elect this node as leader
    #[tracing::instrument(level = "debug", skip(self))]
    pub(crate) fn elect(&mut self) {
        let new_term = self.state.vote.term().next();
        let leader_id = LeaderIdOf::<C>::new(new_term, self.config.id.clone());
        let new_vote = VoteOf::<C>::from_leader_id(leader_id, false);

        let candidate = self.new_candidate(new_vote.clone());

        tracing::info!("{}, new candidate: {}", func_name!(), candidate);

        let last_log_id = candidate.last_log_id().cloned();

        // Simulate sending RequestVote RPC to local node.
        // Safe unwrap(): it won't reject itself ˙–˙
        self.vote_handler().update_vote(&new_vote).unwrap();

        self.output.push_command(Command::SendVote {
            vote_req: VoteRequest::new(new_vote, last_log_id),
        });

        self.server_state_handler().update_server_state_if_changed();
    }

    pub(crate) fn leader_ref(&self) -> Option<&Leader<C, LeaderQuorumSet<C>>> {
        self.leader.as_deref()
    }

    pub(crate) fn leader_mut(&mut self) -> Option<&mut Leader<C, LeaderQuorumSet<C>>> {
        self.leader.as_deref_mut()
    }

    pub(crate) fn candidate_ref(&self) -> Option<&Candidate<C, LeaderQuorumSet<C>>> {
        self.candidate.as_ref()
    }

    pub(crate) fn candidate_mut(&mut self) -> Option<&mut Candidate<C, LeaderQuorumSet<C>>> {
        self.candidate.as_mut()
    }

    /// Get a LeaderHandler for handling leader's operation. If it is not a leader, it sends back a
    /// ForwardToLeader error through the tx.
    ///
    /// If `tx` is None, no response will be sent.
    ///
    /// The `tx` is a [`Responder`] instance. The generic `R` allows any responder type to be used,
    /// while the responder from [`C::Responder`] is specifically designed for client write
    /// operations.
    ///
    /// [`C::Responder`]: RaftTypeConfig::Responder
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn get_leader_handler_or_reject<R>(
        &mut self,
        tx: Option<R>,
    ) -> Option<(LeaderHandler<'_, C>, Option<R>)>
    where
        R: Responder<C, ClientWriteResult<C>>,
    {
        let res = self.leader_handler();
        let forward_err = match res {
            Ok(lh) => {
                tracing::debug!("this node is a leader");
                return Some((lh, tx));
            }
            Err(forward_err) => forward_err,
        };

        if let Some(tx) = tx {
            tx.on_complete(Err(forward_err.into()));
        }

        None
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn handle_vote_req(&mut self, req: VoteRequest<C>) -> VoteResponse<C> {
        let now = C::now();
        let local_leased_vote = &self.state.vote;

        tracing::info!(req = display(&req), "Engine::handle_vote_req");
        tracing::info!(
            my_vote = display(&**local_leased_vote),
            my_last_log_id = display(self.state.last_log_id().display()),
            lease = display(local_leased_vote.display_lease_info(now)),
            "Engine::handle_vote_req"
        );

        if local_leased_vote.is_committed() {
            // Current leader lease has not yet expired, reject voting request
            if !local_leased_vote.is_expired(now, Duration::from_millis(0)) {
                tracing::info!(
                    "reject vote-request: leader lease has not yet expire: {}",
                    local_leased_vote.display_lease_info(now)
                );

                return VoteResponse::new(self.state.vote_ref(), self.state.last_log_id().cloned(), false);
            }
        }

        // The first step is to check log. If the candidate has less log, nothing needs to be done.

        if req.last_log_id.as_ref() >= self.state.last_log_id() {
            // Ok
        } else {
            tracing::info!(
                "reject vote-request: by last_log_id: !(req.last_log_id({}) >= my_last_log_id({})",
                req.last_log_id.display(),
                self.state.last_log_id().display(),
            );
            // The res is not used yet.
            // let _res = Err(RejectVoteRequest::ByLastLogId(self.state.last_log_id().copied()));

            // Return the updated vote, this way the candidate knows which vote is granted, in case
            // the candidate's vote is changed after sending the vote request.
            return VoteResponse::new(self.state.vote_ref(), self.state.last_log_id().cloned(), false);
        }

        // Then check vote just as it does for every incoming event.

        let res = self.vote_handler().update_vote(&req.vote);

        tracing::info!(req = display(&req), result = debug(&res), "handle vote request result");

        // Return the updated vote, this way the candidate knows which vote is granted, in case
        // the candidate's vote is changed after sending the vote request.
        VoteResponse::new(self.state.vote_ref(), self.state.last_log_id().cloned(), res.is_ok())
    }

    #[tracing::instrument(level = "debug", skip(self, resp))]
    pub(crate) fn handle_vote_resp(&mut self, target: C::NodeId, resp: VoteResponse<C>) {
        tracing::info!(
            resp = display(&resp),
            target = display(&target),
            my_vote = display(self.state.vote_ref()),
            my_last_log_id = display(self.state.last_log_id().display()),
            "{}",
            func_name!()
        );

        let Some(candidate) = self.candidate_mut() else {
            // If the voting process has finished or canceled,
            // just ignore the delayed vote_resp.
            return;
        };

        // If resp.vote is different, it may be a delay response to previous voting.
        if resp.vote_granted && &resp.vote == candidate.vote_ref() {
            let quorum_granted = candidate.grant_by(&target);
            if quorum_granted {
                tracing::info!("a quorum granted my vote");
                self.establish_leader();
            }
            return;
        }

        // If not equal, vote is rejected:

        // Note that it is still possible seeing a smaller vote:
        // - The target has more logs than this node;
        // - Or leader lease on remote node is not expired;
        // - It is a delayed response of previous voting(resp.vote_granted could be true)
        // In any case, no need to proceed.

        // Seen a higher log. Record it so that the next election will be delayed for a while.
        if resp.last_log_id.as_ref() > self.state.last_log_id() {
            tracing::info!(
                greater_log_id = display(resp.last_log_id.display()),
                "seen a greater log id when {}",
                func_name!()
            );
            self.set_greater_log();
        }

        // When vote request is rejected, only update to the non-committed version of the vote.
        //
        // This prevents a dangerous scenario when state reversion is allowed:
        // 1. A node was a leader but its state reverted to a previous version
        // 2. The node restarts and begins election
        // 3. It receives a vote response containing its own previous leader vote
        // 4. Without this protection, it would update to that committed vote and become leader again
        // 5. However, it lacks the necessary logs, causing committed entries to be lost or inconsistent
        //
        // By using the non-committed version, we prevent this reverted node from becoming leader
        // while still allowing proper vote updates for legitimate cases.
        let vote = resp.vote.to_non_committed().into_vote();

        // Update if resp.vote is greater.
        let _ = self.vote_handler().update_vote(&vote);
    }

    /// Append entries to follower/learner.
    ///
    /// Also clean conflicting entries and update membership state.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn handle_append_entries(
        &mut self,
        vote: &VoteOf<C>,
        prev_log_id: Option<LogIdOf<C>>,
        entries: Vec<C::Entry>,
        tx: AppendEntriesTx<C>,
    ) -> bool {
        tracing::debug!(
            vote = display(vote),
            prev_log_id = display(prev_log_id.display()),
            entries = display(entries.display()),
            my_vote = display(self.state.vote_ref()),
            my_last_log_id = display(self.state.last_log_id().display()),
            "{}",
            func_name!()
        );

        let res = self.append_entries(vote, prev_log_id, entries);
        let is_ok = res.is_ok();

        let resp: AppendEntriesResponse<C> = res.into();

        let condition = if is_ok {
            Some(Condition::IOFlushed {
                io_id: self.state.accepted_log_io().unwrap().clone(),
            })
        } else {
            None
        };

        self.output.push_command(Command::Respond {
            when: condition,
            resp: Respond::new(resp, tx),
        });

        is_ok
    }

    pub(crate) fn append_entries(
        &mut self,
        vote: &VoteOf<C>,
        prev_log_id: Option<LogIdOf<C>>,
        entries: Vec<C::Entry>,
    ) -> Result<(), RejectAppendEntries<C>> {
        self.vote_handler().update_vote(vote)?;

        // Vote is legal.

        let mut fh = self.following_handler();
        fh.ensure_log_consecutive(prev_log_id.as_ref())?;
        fh.append_entries(prev_log_id, entries);

        Ok(())
    }

    /// Install a completely received snapshot on a follower.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn handle_install_full_snapshot(
        &mut self,
        vote: VoteOf<C>,
        snapshot: Snapshot<C>,
        tx: OneshotSenderOf<C, SnapshotResponse<C>>,
    ) {
        tracing::info!(vote = display(&vote), snapshot = display(&snapshot), "{}", func_name!());

        let vote_res = self.vote_handler().accept_vote(&vote, tx, |state, _rejected| {
            SnapshotResponse::new(state.vote_ref().clone())
        });

        let Some(tx) = vote_res else {
            return;
        };

        let mut fh = self.following_handler();

        // The condition to satisfy before running other command that depends on the snapshot.
        // In this case, the response can only be sent when the snapshot is installed.
        let cond = fh.install_full_snapshot(snapshot);
        let res = SnapshotResponse {
            vote: self.state.vote_ref().clone(),
        };

        self.output.push_command(Command::Respond {
            when: cond,
            resp: Respond::new(res, tx),
        });
    }

    /// Install a completely received snapshot on a follower.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn handle_begin_receiving_snapshot(&mut self, tx: OneshotSenderOf<C, SnapshotDataOf<C>>) {
        tracing::info!("{}", func_name!());
        self.output.push_command(Command::from(sm::Command::begin_receiving_snapshot(tx)));
    }

    /// Leader steps down(convert to learner) once the membership not containing it is committed.
    ///
    /// This is only called by leader.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn leader_step_down(&mut self) {
        tracing::debug!("leader_step_down: node_id:{}", self.config.id);

        // Step down:
        // Keep acting as leader until a membership without this node is committed.
        let em = &self.state.membership_state.effective();

        tracing::debug!(
            "membership: {}, committed: {}, is_leading: {}",
            em,
            self.state.committed().display(),
            self.state.is_leading(&self.config.id),
        );

        #[allow(clippy::collapsible_if)]
        if em.log_id().as_ref() <= self.state.committed() {
            self.vote_handler().update_internal_server_state();
        }
    }

    /// Update Engine state when snapshot building completes or is deferred.
    ///
    /// # Arguments
    ///
    /// - `meta`: The snapshot metadata if building succeeded, or `None` if the state machine
    ///   deferred snapshot creation via `try_create_snapshot_builder()`.
    ///
    /// # Implementation Notes
    ///
    /// Snapshot building runs asynchronously in the background. This creates race conditions:
    /// - While building, a newer snapshot may be installed from the leader
    /// - The installed snapshot may have advanced the snapshot progress beyond this build
    ///
    /// To handle this, we use `try_update_all()` which only updates progress if still behind,
    /// preventing regression of the snapshot cursor.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn on_building_snapshot_done(&mut self, meta: Option<SnapshotMeta<C>>) {
        tracing::info!("{}: snapshot_meta: {}", func_name!(), meta.display());

        self.state.io_state_mut().set_building_snapshot(false);

        let Some(meta) = meta else {
            tracing::info!("snapshot building deferred by state machine, no meta update");
            return;
        };

        // Snapshot building runs asynchronously. While it was building,
        // a newer snapshot may have been installed from the leader,
        // advancing the snapshot progress. Only update if still behind.
        if let Some(last_log_id) = meta.last_log_id.clone() {
            self.state.io_state_mut().snapshot.try_update_all(last_log_id);
        }

        let mut h = self.snapshot_handler();

        let updated = h.update_snapshot(meta);
        if !updated {
            return;
        }

        self.log_handler().schedule_policy_based_purge();
        self.try_purge_log();
    }

    /// Try to purge logs up to the expected position.
    ///
    /// If the node is a leader, it will only purge logs when no replication tasks are using them.
    /// Otherwise, it will retry purging the logs the next time replication has made progress.
    ///
    /// If the node is a follower or learner, it will always purge the logs immediately since no
    /// other tasks are using the logs.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn try_purge_log(&mut self) {
        tracing::debug!(
            purge_upto = display(self.state.purge_upto().display()),
            "{}",
            func_name!()
        );

        if self.leader.is_some() {
            // If it is leading, it must not delete a log that is in use by a replication task.
            self.replication_handler().try_purge_log();
        } else {
            // For follower/learner, no other tasks are using logs, just purge.
            self.log_handler().purge_log();
        }
    }

    /// This is a to user API that triggers log purging up to `index`, inclusive.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn trigger_purge_log(&mut self, mut index: u64) {
        tracing::info!(index = display(index), "{}", func_name!());

        let snapshot_last_log_id = self.state.snapshot_last_log_id();
        let snapshot_last_log_id = if let Some(log_id) = snapshot_last_log_id {
            log_id.clone()
        } else {
            tracing::info!("no snapshot, cannot purge");
            return;
        };

        let scheduled = self.state.purge_upto();

        if index < scheduled.next_index() {
            tracing::info!(
                "no update, already scheduled: {}; index: {}",
                scheduled.display(),
                index,
            );
            return;
        }

        if index > snapshot_last_log_id.index() {
            tracing::info!(
                "cannot purge logs not in a snapshot; index: {}, last in snapshot log id: {}",
                index,
                snapshot_last_log_id
            );
            index = snapshot_last_log_id.index();
        }

        // Safe unwrap: `index` is ensured to be present in the above code.
        let log_id = self.state.get_log_id(index).unwrap();

        tracing::info!(purge_upto = display(&log_id), "{}", func_name!());

        self.log_handler().update_purge_upto(log_id);
        self.try_purge_log();
    }

    pub(crate) fn trigger_transfer_leader(&mut self, to: C::NodeId) {
        tracing::info!(to = display(&to), "{}", func_name!());

        let Some((mut lh, _)) = self.get_leader_handler_or_reject(None::<WriteResponderOf<C>>) else {
            tracing::info!(
                to = display(to),
                "{}: this node is not a Leader, ignore transfer Leader",
                func_name!()
            );
            return;
        };

        lh.transfer_leader(to);
    }

    /// Poll for commands automatically generated from I/O progress state.
    ///
    /// Returns commands built by examining I/O progress (submitted vs accepted positions),
    /// without explicit Engine state changes. These commands have no preconditions and can be
    /// executed immediately without queuing.
    ///
    /// Currently, generates [`Command::SaveCommittedAndApply`] when committed log entries
    /// haven't been applied: `(apply_progress.submitted()..apply_progress.accepted()]`.
    ///
    /// Requirements:
    /// - Commands must update their corresponding progress when executed to prevent duplicates
    pub(crate) fn next_progress_driven_command(&self) -> Option<Command<C>> {
        let apply_progress = &self.state.io_state.apply_progress;
        let log_progress = &self.state.io_state.log_progress;

        // Generate Apply command

        if log_progress.submitted().map(|x| x.as_ref_vote()) == log_progress.accepted().map(|x| x.as_ref_vote()) {
            // Only apply committed entries when submitted and accepted logs are from the same leader.
            //
            // This ensures submitted logs won't be overridden by pending commands in the queue.
            //
            // If leaders differ, queued commands may override submitted logs. Example:
            // - submitted: append-entries(leader=L1, entry=E2)
            // - queued: truncate(E2), save-vote(L2), append-entries(leader=L2, entry=E2')
            // Here E2 will be overridden by E2' when the queue executes.
            //
            // When both have the same leader:
            // - A leader never truncates its own written entries
            // - Committed entries are visible to all future leaders
            // - The submitted logs are guaranteed to be the actual committed entries

            let apply_submitted = apply_progress.submitted();
            let apply_accepted = apply_progress.accepted();

            let log_submitted = log_progress.submitted().and_then(|io_id| io_id.last_log_id());

            let applicable_upto = log_submitted.min(apply_accepted);

            if apply_submitted.next_index() < applicable_upto.next_index() {
                let apply_upto = applicable_upto.cloned().unwrap();

                return Some(Command::SaveCommittedAndApply {
                    already_applied: apply_submitted.cloned(),
                    upto: apply_upto,
                });
            }
        }

        None
    }
}

/// Supporting util
impl<C> Engine<C>
where C: RaftTypeConfig
{
    /// Vote is granted by a quorum, leader established.
    #[tracing::instrument(level = "debug", skip_all)]
    fn establish_leader(&mut self) {
        tracing::info!("{}", func_name!());

        let candidate = self.candidate.take().unwrap();
        let leader = self.establish_handler().establish(candidate);

        // There may already be a Leader with higher vote
        let Some(leader) = leader else { return };

        let vote = leader.committed_vote_ref().clone();
        let last_log_id = leader.last_log_id().cloned();

        self.replication_handler().rebuild_replication_streams();

        // Before sending any log, update the vote.
        // This could not fail because `internal_server_state` will be cleared
        // once `state.vote` is changed to a value of other node.
        let _res = self.vote_handler().update_vote(&vote.clone().into_vote());
        debug_assert!(_res.is_ok(), "commit vote cannot fail but: {:?}", _res);

        self.state.accept_log_io(IOId::new_log_io(vote, last_log_id));

        // No need to submit UpdateIOProgress command,
        // IO progress is updated by the new blank log

        self.leader_handler()
            .unwrap()
            .leader_append_entries(vec![C::Entry::new_blank(LogIdOf::<C>::default())]);
    }

    /// Check if a raft node is in a state that allows to initialize.
    ///
    /// It is allowed to initialize only when `last_log_id.is_none()` and `vote==(term=0,
    /// node_id=0)`. See: [Conditions for initialization](https://databendlabs.github.io/openraft/cluster-formation.html#conditions-for-initialization)
    fn check_initialize(&self) -> Result<(), NotAllowed<C>> {
        if !self.state.is_initialized() {
            return Ok(());
        }

        tracing::info!(
            last_log_id = display(self.state.last_log_id().display()),
            vote = display(self.state.vote_ref()),
            "Engine::check_initialize(): cannot initialize"
        );

        Err(NotAllowed {
            last_log_id: self.state.last_log_id().cloned(),
            vote: self.state.vote_ref().clone(),
        })
    }

    /// When initialized, the node that accept initialize request has to be a member of the initial
    /// config.
    fn check_members_contain_me(&self, m: &Membership<C>) -> Result<(), NotInMembers<C>> {
        if !m.is_voter(&self.config.id) {
            let e = NotInMembers {
                node_id: self.config.id.clone(),
                membership: m.clone(),
            };
            Err(e)
        } else {
            Ok(())
        }
    }

    pub(crate) fn is_there_greater_log(&self) -> bool {
        self.seen_greater_log
    }

    /// Set that there is greater last log id found.
    ///
    /// In such a case, this node should not try to elect aggressively.
    pub(crate) fn set_greater_log(&mut self) {
        self.seen_greater_log = true;
    }

    /// Clear the flag of that there is greater last log id.
    pub(crate) fn reset_greater_log(&mut self) {
        self.seen_greater_log = false;
    }

    // Only used by tests
    #[allow(dead_code)]
    pub(crate) fn calc_server_state(&self) -> ServerState {
        self.state.calc_server_state(&self.config.id)
    }

    // --- handlers ---

    pub(crate) fn vote_handler(&mut self) -> VoteHandler<'_, C> {
        VoteHandler {
            config: &mut self.config,
            state: &mut self.state,
            output: &mut self.output,
            leader: &mut self.leader,
            candidate: &mut self.candidate,
        }
    }

    pub(crate) fn log_handler(&mut self) -> LogHandler<'_, C> {
        LogHandler {
            config: &mut self.config,
            state: &mut self.state,
            output: &mut self.output,
        }
    }

    pub(crate) fn snapshot_handler(&mut self) -> SnapshotHandler<'_, '_, C> {
        SnapshotHandler {
            state: &mut self.state,
            output: &mut self.output,
        }
    }

    pub(crate) fn leader_handler(&mut self) -> Result<LeaderHandler<'_, C>, ForwardToLeader<C>> {
        let leader = match self.leader.as_mut() {
            None => {
                tracing::debug!("this node is NOT a leader: {:?}", self.state.server_state);
                return Err(self.state.forward_to_leader());
            }
            Some(x) => x,
        };

        debug_assert!(
            leader.committed_vote_ref().as_ref_vote() >= self.state.vote_ref().as_ref_vote(),
            "leader.vote({}) >= state.vote({})",
            leader.committed_vote_ref(),
            self.state.vote_ref()
        );

        Ok(LeaderHandler {
            config: &mut self.config,
            leader,
            state: &mut self.state,
            output: &mut self.output,
        })
    }

    pub(crate) fn replication_handler(&mut self) -> ReplicationHandler<'_, C> {
        let leader = match self.leader.as_mut() {
            None => {
                unreachable!("There is no leader, cannot handle replication");
            }
            Some(x) => x,
        };

        ReplicationHandler {
            config: &mut self.config,
            leader,
            state: &mut self.state,
            output: &mut self.output,
        }
    }

    pub(crate) fn following_handler(&mut self) -> FollowingHandler<'_, C> {
        debug_assert!(self.leader.is_none());

        let leader_vote = self.state.vote_ref().clone();
        debug_assert!(
            leader_vote.is_committed(),
            "Expect the Leader vote to be committed: {}",
            leader_vote
        );

        FollowingHandler {
            leader_vote: leader_vote.into_committed(),
            config: &mut self.config,
            state: &mut self.state,
            output: &mut self.output,
        }
    }

    pub(crate) fn server_state_handler(&mut self) -> ServerStateHandler<'_, C> {
        ServerStateHandler {
            config: &self.config,
            state: &mut self.state,
        }
    }
    pub(crate) fn establish_handler(&mut self) -> EstablishHandler<'_, C> {
        EstablishHandler {
            config: &mut self.config,
            leader: &mut self.leader,
        }
    }
}

/// Supporting utilities for unit test
#[cfg(test)]
mod engine_testing {
    use crate::RaftTypeConfig;
    use crate::engine::Engine;
    use crate::proposer::LeaderQuorumSet;

    impl<C> Engine<C>
    where C: RaftTypeConfig
    {
        /// Create a Leader state just for testing purpose only,
        /// without initializing related resource,
        /// such as setting up replication, propose blank log.
        pub(crate) fn testing_new_leader(&mut self) -> &mut crate::proposer::Leader<C, LeaderQuorumSet<C>> {
            let leader = self.state.new_leader();
            self.leader = Some(Box::new(leader));
            self.leader.as_mut().unwrap()
        }
    }
}
