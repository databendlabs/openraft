use std::time::Duration;

use validit::Valid;

use crate::core::raft_msg::AppendEntriesTx;
use crate::core::raft_msg::InstallSnapshotTx;
use crate::core::raft_msg::ResultSender;
use crate::core::ServerState;
use crate::display_ext::DisplayOptionExt;
use crate::display_ext::DisplaySlice;
use crate::engine::engine_config::EngineConfig;
use crate::engine::handler::following_handler::FollowingHandler;
use crate::engine::handler::leader_handler::LeaderHandler;
use crate::engine::handler::log_handler::LogHandler;
use crate::engine::handler::replication_handler::ReplicationHandler;
use crate::engine::handler::replication_handler::SendNone;
use crate::engine::handler::server_state_handler::ServerStateHandler;
use crate::engine::handler::snapshot_handler::SnapshotHandler;
use crate::engine::handler::vote_handler::VoteHandler;
use crate::engine::Command;
use crate::engine::Condition;
use crate::engine::EngineOutput;
use crate::engine::Respond;
use crate::entry::RaftPayload;
use crate::error::ForwardToLeader;
use crate::error::InitializeError;
use crate::error::InstallSnapshotError;
use crate::error::NotAllowed;
use crate::error::NotInMembers;
use crate::error::RejectAppendEntries;
use crate::internal_server_state::InternalServerState;
use crate::membership::EffectiveMembership;
use crate::raft::AppendEntriesResponse;
use crate::raft::InstallSnapshotRequest;
use crate::raft::InstallSnapshotResponse;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::raft_state::LogStateReader;
use crate::raft_state::RaftState;
use crate::summary::MessageSummary;
use crate::AsyncRuntime;
use crate::Instant;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::Membership;
use crate::RaftLogId;
use crate::RaftTypeConfig;
use crate::SnapshotMeta;
use crate::Vote;

/// Raft protocol algorithm.
///
/// It implement the complete raft algorithm except does not actually update any states.
/// But instead, it output commands to let a `RaftRuntime` implementation execute them to actually
/// update the states such as append-log or save-vote by execute .
///
/// This structure only contains necessary information to run raft algorithm,
/// but none of the application specific data.
/// TODO: make the fields private
#[derive(Debug, Default)]
pub(crate) struct Engine<C>
where C: RaftTypeConfig
{
    pub(crate) config: EngineConfig<C::NodeId>,

    /// The state of this raft node.
    pub(crate) state: Valid<RaftState<C::NodeId, C::Node, <C::AsyncRuntime as AsyncRuntime>::Instant>>,

    // TODO: add a Voting state as a container.
    /// Whether a greater log id is seen during election.
    ///
    /// If it is true, then this node **may** not become a leader therefore the election timeout
    /// should be greater.
    pub(crate) seen_greater_log: bool,

    /// The internal server state used by Engine.
    pub(crate) internal_server_state: InternalServerState<C::NodeId, <C::AsyncRuntime as AsyncRuntime>::Instant>,

    /// Output entry for the runtime.
    pub(crate) output: EngineOutput<C>,
}

impl<C> Engine<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(
        init_state: RaftState<C::NodeId, C::Node, <C::AsyncRuntime as AsyncRuntime>::Instant>,
        config: EngineConfig<C::NodeId>,
    ) -> Self {
        Self {
            config,
            state: Valid::new(init_state),
            seen_greater_log: false,
            internal_server_state: InternalServerState::default(),
            output: EngineOutput::new(4096),
        }
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn startup(&mut self) {
        // Allows starting up as a leader.

        tracing::info!("startup: state: {:?}", self.state);
        tracing::info!("startup: is_leader: {}", self.state.is_leader(&self.config.id));
        tracing::info!(
            "startup: is_voter: {}",
            self.state.membership_state.effective().is_voter(&self.config.id)
        );

        // Previously it is a leader. restore it as leader at once
        if self.state.is_leader(&self.config.id) {
            self.vote_handler().update_internal_server_state();

            let mut rh = self.replication_handler();
            rh.rebuild_replication_streams();

            // Restore the progress about the local log
            rh.update_local_progress(rh.state.last_log_id().copied());

            rh.initiate_replication(SendNone::False);

            return;
        }

        let server_state = if self.state.membership_state.effective().is_voter(&self.config.id) {
            ServerState::Follower
        } else {
            ServerState::Learner
        };

        self.state.server_state = server_state;

        tracing::info!(
            "startup: id={} target_state: {:?}",
            self.config.id,
            self.state.server_state
        );
    }

    /// Initialize a node by appending the first log.
    ///
    /// - The first log has to be membership config log.
    /// - The node has to contain no logs at all and the vote is the minimal value. See: [Conditions
    ///   for initialization](https://datafuselabs.github.io/openraft/cluster-formation.html#conditions-for-initialization)
    ///
    /// Appending the very first log is slightly different from appending log by a leader or
    /// follower. This step is not confined by the consensus protocol and has to be dealt with
    /// differently.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn initialize(&mut self, mut entry: C::Entry) -> Result<(), InitializeError<C::NodeId, C::Node>> {
        self.check_initialize()?;

        self.state.assign_log_ids([&mut entry]);
        let log_id = *entry.get_log_id();
        self.state.extend_log_ids_from_same_leader(&[log_id]);

        let m = entry.get_membership().expect("the only log entry for initializing has to be membership log");
        self.check_members_contain_me(m)?;

        tracing::debug!("update effective membership: log_id:{} {}", log_id, m.summary());

        let em = EffectiveMembership::new_arc(Some(log_id), m.clone());
        self.state.membership_state.append(em);

        self.output.push_command(Command::AppendEntry { entry });

        self.server_state_handler().update_server_state_if_changed();

        // With the new config, start to elect to become leader
        self.elect();

        Ok(())
    }

    /// Start to elect this node as leader
    #[tracing::instrument(level = "debug", skip(self))]
    pub(crate) fn elect(&mut self) {
        let v = Vote::new(self.state.vote_ref().leader_id().term + 1, self.config.id);
        tracing::info!(vote = display(&v), "{}", func_name!());

        // Safe unwrap(): it won't reject itself ˙–˙
        self.vote_handler().update_vote(&v).unwrap();

        // TODO: simplify voting initialization.
        //       - update_vote() should be moved to after initialize_voting(), because it can be considered
        //         as a local RPC

        // Safe unwrap(): leading state is just created
        let leading = self.internal_server_state.leading_mut().unwrap();
        let voting = leading.initialize_voting(
            self.state.last_log_id().copied(),
            <C::AsyncRuntime as AsyncRuntime>::Instant::now(),
        );

        let quorum_granted = voting.grant_by(&self.config.id);

        // Fast-path: if there is only one voter in the cluster.

        if quorum_granted {
            self.establish_leader();
            return;
        }

        // Slow-path: send vote request, let a quorum grant it.

        self.output.push_command(Command::SendVote {
            vote_req: VoteRequest::new(*self.state.vote_ref(), self.state.last_log_id().copied()),
        });

        self.server_state_handler().update_server_state_if_changed();
    }

    /// Get a LeaderHandler for handling leader's operation. If it is not a leader, it send back a
    /// ForwardToLeader error through the tx.
    ///
    /// If tx is None, no response will be sent.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn get_leader_handler_or_reject<T, E>(
        &mut self,
        tx: Option<ResultSender<T, E>>,
    ) -> Option<(LeaderHandler<C>, Option<ResultSender<T, E>>)>
    where
        E: From<ForwardToLeader<C::NodeId, C::Node>>,
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
            let _ = tx.send(Err(forward_err.into()));
        }

        None
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn handle_vote_req(&mut self, req: VoteRequest<C::NodeId>) -> VoteResponse<C::NodeId> {
        let now = <C::AsyncRuntime as AsyncRuntime>::Instant::now();
        let lease = self.config.timer_config.leader_lease;
        let vote = self.state.vote_ref();

        // Make default vote-last-modified a low enough value, that expires leader lease.
        let vote_utime = self.state.vote_last_modified().unwrap_or_else(|| now - lease - Duration::from_millis(1));

        tracing::info!(req = display(req.summary()), "Engine::handle_vote_req");
        tracing::info!(
            my_vote = display(self.state.vote_ref().summary()),
            my_last_log_id = display(self.state.last_log_id().summary()),
            "Engine::handle_vote_req"
        );
        tracing::info!(
            "now; {:?}, vote is updated at: {:?}, vote is updated before {:?}, leader lease({:?}) will expire after {:?}",
            now,
            vote_utime,
            now- vote_utime,
            lease,
            vote_utime + lease - now
        );

        if vote.is_committed() {
            // Current leader lease has not yet expired, reject voting request
            if now <= vote_utime + lease {
                tracing::info!(
                    "reject vote-request: leader lease has not yet expire; now; {:?}, vote is updatd at: {:?}, leader lease({:?}) will expire after {:?}",
                    now,
                    vote_utime,
                    lease,
                    vote_utime + lease - now
                );

                return VoteResponse {
                    vote: *self.state.vote_ref(),
                    vote_granted: false,
                    last_log_id: self.state.last_log_id().copied(),
                };
            }
        }

        // The first step is to check log. If the candidate has less log, nothing needs to be done.

        if req.last_log_id.as_ref() >= self.state.last_log_id() {
            // Ok
        } else {
            tracing::info!(
                "reject vote-request: by last_log_id: !(req.last_log_id({}) >= my_last_log_id({})",
                req.last_log_id.summary(),
                self.state.last_log_id().summary(),
            );
            // The res is not used yet.
            // let _res = Err(RejectVoteRequest::ByLastLogId(self.state.last_log_id().copied()));
            return VoteResponse {
                // Return the updated vote, this way the candidate knows which vote is granted, in case
                // the candidate's vote is changed after sending the vote request.
                vote: *self.state.vote_ref(),
                vote_granted: false,
                last_log_id: self.state.last_log_id().copied(),
            };
        }

        // Then check vote just as it does for every incoming event.

        let res = self.vote_handler().update_vote(&req.vote);

        tracing::info!(
            req = display(req.summary()),
            result = debug(&res),
            "handle vote request result"
        );

        let vote_granted = res.is_ok();

        VoteResponse {
            // Return the updated vote, this way the candidate knows which vote is granted, in case
            // the candidate's vote is changed after sending the vote request.
            vote: *self.state.vote_ref(),
            vote_granted,
            last_log_id: self.state.last_log_id().copied(),
        }
    }

    #[tracing::instrument(level = "debug", skip(self, resp))]
    pub(crate) fn handle_vote_resp(&mut self, target: C::NodeId, resp: VoteResponse<C::NodeId>) {
        tracing::info!(
            resp = display(resp.summary()),
            target = display(target),
            my_vote = display(self.state.vote_ref()),
            my_last_log_id = display(self.state.last_log_id().summary()),
            "{}",
            func_name!()
        );

        let voting = if let Some(voting) = self.internal_server_state.voting_mut() {
            // TODO check the sending vote matches current vote
            voting
        } else {
            // If this node is no longer a leader(i.e., electing) or candidate,
            // just ignore the delayed vote_resp.
            return;
        };

        if &resp.vote < self.state.vote_ref() {
            debug_assert!(!resp.vote_granted);
        }

        if resp.vote_granted {
            let quorum_granted = voting.grant_by(&target);
            if quorum_granted {
                tracing::info!("a quorum granted my vote");
                self.establish_leader();
            }
            return;
        }

        // vote is rejected:

        debug_assert!(self.state.membership_state.effective().is_voter(&self.config.id));

        // If peer's vote is greater than current vote, revert to follower state.
        //
        // Explicitly ignore the returned error:
        // resp.vote being not greater than mine is all right.
        let _ = self.vote_handler().update_vote(&resp.vote);

        // Seen a higher log. Record it so that the next election will be delayed for a while.
        if resp.last_log_id.as_ref() > self.state.last_log_id() {
            tracing::info!(
                greater_log_id = display(resp.last_log_id.summary()),
                "seen a greater log id when {}",
                func_name!()
            );
            self.set_greater_log();
        }
    }

    /// Append entries to follower/learner.
    ///
    /// Also clean conflicting entries and update membership state.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn handle_append_entries(
        &mut self,
        vote: &Vote<C::NodeId>,
        prev_log_id: Option<LogId<C::NodeId>>,
        entries: Vec<C::Entry>,
        tx: Option<AppendEntriesTx<C::NodeId>>,
    ) -> bool {
        tracing::debug!(
            vote = display(vote),
            prev_log_id = display(prev_log_id.summary()),
            entries = display(DisplaySlice::<_>(&entries)),
            my_vote = display(self.state.vote_ref()),
            my_last_log_id = display(self.state.last_log_id().summary()),
            "{}",
            func_name!()
        );

        let res = self.append_entries(vote, prev_log_id, entries);
        let is_ok = res.is_ok();

        if let Some(tx) = tx {
            let resp: AppendEntriesResponse<C::NodeId> = res.into();
            self.output.push_command(Command::Respond {
                when: None,
                resp: Respond::new(Ok(resp), tx),
            });
        }
        is_ok
    }

    pub(crate) fn append_entries(
        &mut self,
        vote: &Vote<C::NodeId>,
        prev_log_id: Option<LogId<C::NodeId>>,
        entries: Vec<C::Entry>,
    ) -> Result<(), RejectAppendEntries<C::NodeId>> {
        self.vote_handler().update_vote(vote)?;

        // Vote is legal.

        let mut fh = self.following_handler();
        fh.ensure_log_consecutive(prev_log_id)?;
        fh.append_entries(prev_log_id, entries);

        Ok(())
    }

    /// Commit entries for follower/learner.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn handle_commit_entries(&mut self, leader_committed: Option<LogId<C::NodeId>>) {
        tracing::debug!(
            leader_committed = display(leader_committed.summary()),
            my_accepted = display(self.state.accepted().summary()),
            my_committed = display(self.state.committed().summary()),
            "{}",
            func_name!()
        );

        let mut fh = self.following_handler();
        fh.commit_entries(leader_committed);
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn handle_install_snapshot(
        &mut self,
        req: InstallSnapshotRequest<C>,
        tx: InstallSnapshotTx<C::NodeId>,
    ) -> Option<()> {
        tracing::info!(req = display(req.summary()), "{}", func_name!());

        let res = self.vote_handler().accept_vote(&req.vote, tx, |state, _rejected| {
            Ok(InstallSnapshotResponse {
                vote: *state.vote_ref(),
            })
        });

        let tx = match res {
            Some(tx) => tx,
            None => return Some(()),
        };

        let res = self.install_snapshot(req);
        let res = res.map(|_| InstallSnapshotResponse {
            vote: *self.state.vote_ref(),
        });

        self.output.push_command(Command::Respond {
            // When there is an error, there may still be queued IO, we need to run them before sending back
            // response.
            when: Some(Condition::StateMachineCommand {
                command_seq: self.output.last_sm_seq(),
            }),
            resp: Respond::new(res, tx),
        });

        Some(())
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn install_snapshot(&mut self, req: InstallSnapshotRequest<C>) -> Result<(), InstallSnapshotError> {
        tracing::info!(req = display(req.summary()), "{}", func_name!());

        let done = req.done;
        let snapshot_meta = req.meta.clone();

        let mut fh = self.following_handler();

        fh.receive_snapshot_chunk(req)?;

        if done {
            fh.install_snapshot(snapshot_meta);
        }

        Ok(())
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
            em.summary(),
            self.state.committed().summary(),
            self.state.is_leading(&self.config.id),
        );

        #[allow(clippy::collapsible_if)]
        if em.log_id().as_ref() <= self.state.committed() {
            self.vote_handler().update_internal_server_state();
        }
    }

    /// Update Engine state when a new snapshot is built.
    ///
    /// NOTE:
    /// - Engine updates its state for building a snapshot is done after storage finished building a
    ///   snapshot,
    /// - while Engine updates its state for installing a snapshot is done before storage starts
    ///   installing a snapshot.
    ///
    /// This is all right because:
    /// - Engine only keeps the snapshot meta with the greatest last-log-id;
    /// - and a snapshot smaller than last-committed is not allowed to be installed.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn finish_building_snapshot(&mut self, meta: SnapshotMeta<C::NodeId, C::Node>) {
        tracing::info!(snapshot_meta = display(&meta), "{}", func_name!());

        self.state.io_state_mut().set_building_snapshot(false);

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

        if self.internal_server_state.is_leading() {
            // If it is leading, it must not delete a log that is in use by a replication task.
            self.replication_handler().try_purge_log();
        } else {
            // For follower/learner, no other tasks are using logs, just purge.
            self.log_handler().purge_log();
        }
    }

    /// This is a to user API that triggers log purging upto `index`, inclusive.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn trigger_purge_log(&mut self, mut index: u64) {
        tracing::info!(index = display(index), "{}", func_name!());

        let snapshot_last_log_id = self.state.snapshot_last_log_id();
        let snapshot_last_log_id = if let Some(x) = snapshot_last_log_id {
            *x
        } else {
            tracing::info!("no snapshot, can not purge");
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

        if index > snapshot_last_log_id.index {
            tracing::info!(
                "can not purge logs not in a snapshot; index: {}, last in snapshot log id: {}",
                index,
                snapshot_last_log_id
            );
            index = snapshot_last_log_id.index;
        }

        // Safe unwrap: `index` is ensured to be present in the above code.
        let log_id = self.state.get_log_id(index).unwrap();

        tracing::info!(purge_upto = display(log_id), "{}", func_name!());

        self.log_handler().update_purge_upto(log_id);
        self.try_purge_log();
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

        // Mark the vote as committed, i.e., being granted and saved by a quorum.
        //
        // The committed vote, is not necessary in original raft.
        // Openraft insists doing this because:
        // - Voting is not in the hot path, thus no performance penalty.
        // - Leadership won't be lost if a leader restarted quick enough.
        {
            let leading = self.internal_server_state.leading_mut().unwrap();
            let voting = leading.finish_voting();
            let mut vote = *voting.vote_ref();

            debug_assert!(!vote.is_committed());
            debug_assert_eq!(
                vote.leader_id().voted_for(),
                Some(self.config.id),
                "it can only commit its own vote"
            );
            vote.commit();

            let _res = self.vote_handler().update_vote(&vote);
            debug_assert!(_res.is_ok(), "commit vote can not fail but: {:?}", _res);
        }

        let mut rh = self.replication_handler();

        // It has to setup replication stream first because append_blank_log() may update the
        // committed-log-id(a single leader with several learners), in which case the
        // committed-log-id will be at once submitted to replicate before replication stream
        // is built.
        //
        // TODO: But replication streams should be built when a node enters leading state.
        //       Thus append_blank_log() can be moved before rebuild_replication_streams()

        rh.rebuild_replication_streams();
        rh.append_blank_log();
        rh.initiate_replication(SendNone::False);
    }

    /// Check if a raft node is in a state that allows to initialize.
    ///
    /// It is allowed to initialize only when `last_log_id.is_none()` and `vote==(term=0,
    /// node_id=0)`. See: [Conditions for initialization](https://datafuselabs.github.io/openraft/cluster-formation.html#conditions-for-initialization)
    fn check_initialize(&self) -> Result<(), NotAllowed<C::NodeId>> {
        if self.state.last_log_id().is_none() && self.state.vote_ref() == &Vote::default() {
            return Ok(());
        }

        tracing::error!(
            last_log_id = display(self.state.last_log_id().summary()),
            vote = display(self.state.vote_ref()),
            "Can not initialize"
        );

        Err(NotAllowed {
            last_log_id: self.state.last_log_id().copied(),
            vote: *self.state.vote_ref(),
        })
    }

    /// When initialize, the node that accept initialize request has to be a member of the initial
    /// config.
    fn check_members_contain_me(
        &self,
        m: &Membership<C::NodeId, C::Node>,
    ) -> Result<(), NotInMembers<C::NodeId, C::Node>> {
        if !m.is_voter(&self.config.id) {
            let e = NotInMembers {
                node_id: self.config.id,
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

    pub(crate) fn vote_handler(&mut self) -> VoteHandler<C> {
        VoteHandler {
            config: &self.config,
            state: &mut self.state,
            output: &mut self.output,
            internal_server_state: &mut self.internal_server_state,
        }
    }

    pub(crate) fn log_handler(&mut self) -> LogHandler<C> {
        LogHandler {
            config: &mut self.config,
            state: &mut self.state,
            output: &mut self.output,
        }
    }

    pub(crate) fn snapshot_handler(&mut self) -> SnapshotHandler<C> {
        SnapshotHandler {
            state: &mut self.state,
            output: &mut self.output,
        }
    }

    pub(crate) fn leader_handler(&mut self) -> Result<LeaderHandler<C>, ForwardToLeader<C::NodeId, C::Node>> {
        let leader = match self.internal_server_state.leading_mut() {
            None => {
                tracing::debug!("this node is NOT a leader: {:?}", self.state.server_state);
                return Err(self.state.forward_to_leader());
            }
            Some(x) => x,
        };

        if !self.state.is_leader(&self.config.id) {
            return Err(self.state.forward_to_leader());
        }

        Ok(LeaderHandler {
            config: &mut self.config,
            leader,
            state: &mut self.state,
            output: &mut self.output,
        })
    }

    pub(crate) fn replication_handler(&mut self) -> ReplicationHandler<C> {
        let leader = match self.internal_server_state.leading_mut() {
            None => {
                unreachable!("There is no leader, can not handle replication");
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

    pub(crate) fn following_handler(&mut self) -> FollowingHandler<C> {
        debug_assert!(self.internal_server_state.is_following());

        FollowingHandler {
            config: &mut self.config,
            state: &mut self.state,
            output: &mut self.output,
        }
    }

    pub(crate) fn server_state_handler(&mut self) -> ServerStateHandler<C> {
        ServerStateHandler {
            config: &self.config,
            state: &mut self.state,
            output: &mut self.output,
        }
    }
}
