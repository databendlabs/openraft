use std::time::Duration;

use tokio::time::Instant;

use crate::core::ServerState;
use crate::display_ext::DisplaySlice;
use crate::engine::handler::following_handler::FollowingHandler;
use crate::engine::handler::leader_handler::LeaderHandler;
use crate::engine::handler::log_handler::LogHandler;
use crate::engine::handler::replication_handler::ReplicationHandler;
use crate::engine::handler::replication_handler::SendNone;
use crate::engine::handler::server_state_handler::ServerStateHandler;
use crate::engine::handler::snapshot_handler::SnapshotHandler;
use crate::engine::handler::vote_handler::VoteHandler;
use crate::engine::time_state;
use crate::engine::time_state::TimeState;
use crate::engine::Command;
use crate::entry::RaftEntry;
use crate::error::ForwardToLeader;
use crate::error::InitializeError;
use crate::error::NotAllowed;
use crate::error::NotInMembers;
use crate::internal_server_state::InternalServerState;
use crate::membership::EffectiveMembership;
use crate::node::Node;
use crate::raft::AppendEntriesResponse;
use crate::raft::RaftRespTx;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::raft_state::LogStateReader;
use crate::raft_state::RaftState;
use crate::summary::MessageSummary;
use crate::validate::Valid;
use crate::Config;
use crate::LogId;
use crate::Membership;
use crate::MetricsChangeFlags;
use crate::NodeId;
use crate::SnapshotMeta;
use crate::Vote;

/// Config for Engine
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
pub(crate) struct EngineConfig<NID: NodeId> {
    /// The id of this node.
    pub(crate) id: NID,

    /// The maximum number of applied logs to keep before purging.
    pub(crate) max_in_snapshot_log_to_keep: u64,

    /// The minimal number of applied logs to purge in a batch.
    pub(crate) purge_batch_size: u64,

    /// The maximum number of entries per payload allowed to be transmitted during replication
    pub(crate) max_payload_entries: u64,

    pub(crate) timer_config: time_state::Config,
}

impl<NID: NodeId> Default for EngineConfig<NID> {
    fn default() -> Self {
        Self {
            id: NID::default(),
            max_in_snapshot_log_to_keep: 1000,
            purge_batch_size: 256,
            max_payload_entries: 300,
            timer_config: time_state::Config::default(),
        }
    }
}

impl<NID: NodeId> EngineConfig<NID> {
    pub(crate) fn new(id: NID, config: &Config) -> Self {
        let election_timeout = Duration::from_millis(config.new_rand_election_timeout());
        Self {
            id,
            max_in_snapshot_log_to_keep: config.max_in_snapshot_log_to_keep,
            purge_batch_size: config.purge_batch_size,
            max_payload_entries: config.max_payload_entries,
            timer_config: time_state::Config {
                election_timeout,
                smaller_log_timeout: Duration::from_millis(config.election_timeout_max * 2),
                leader_lease: Duration::from_millis(config.election_timeout_max),
            },
        }
    }
}

/// The entry of output from Engine to the runtime.
#[derive(Debug, Default)]
#[derive(PartialEq, Eq)]
pub(crate) struct EngineOutput<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Tracks what kind of metrics changed
    pub(crate) metrics_flags: MetricsChangeFlags,

    /// Command queue that need to be executed by `RaftRuntime`.
    pub(crate) commands: Vec<Command<NID, N>>,
}

impl<NID, N> EngineOutput<NID, N>
where
    NID: NodeId,
    N: Node,
{
    pub(crate) fn push_command(&mut self, cmd: Command<NID, N>) {
        cmd.update_metrics_flags(&mut self.metrics_flags);
        self.commands.push(cmd)
    }
}

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
#[derive(PartialEq, Eq)]
pub(crate) struct Engine<NID, N>
where
    NID: NodeId,
    N: Node,
{
    pub(crate) config: EngineConfig<NID>,

    /// The state of this raft node.
    pub(crate) state: Valid<RaftState<NID, N>>,

    // TODO: add a Voting state as a container.
    /// Whether a greater log id is seen during election.
    ///
    /// If it is true, then this node **may** not become a leader therefore the election timeout
    /// should be greater.
    pub(crate) seen_greater_log: bool,

    pub(crate) timer: TimeState,

    /// The internal server state used by Engine.
    pub(crate) internal_server_state: InternalServerState<NID>,

    /// Output entry for the runtime.
    pub(crate) output: EngineOutput<NID, N>,
}

impl<NID, N> Engine<NID, N>
where
    N: Node,
    NID: NodeId,
{
    pub(crate) fn new(init_state: RaftState<NID, N>, config: EngineConfig<NID>) -> Self {
        let now = Instant::now();
        Self {
            config,
            state: Valid::new(init_state),
            seen_greater_log: false,
            timer: time_state::TimeState::new(now),
            internal_server_state: InternalServerState::default(),
            output: EngineOutput::default(),
        }
    }

    // TODO: test it
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
    #[tracing::instrument(level = "debug", skip(self, entries))]
    pub(crate) fn initialize<Ent: RaftEntry<NID, N>>(
        &mut self,
        entries: &mut [Ent],
    ) -> Result<(), InitializeError<NID, N>> {
        let l = entries.len();
        debug_assert_eq!(1, l);

        self.check_initialize()?;

        self.state.assign_log_ids(entries.iter_mut());
        self.state.extend_log_ids_from_same_leader(entries);

        self.output.push_command(Command::AppendInputEntries { range: 0..l });

        let entry = &mut entries[0];
        let m = entry.get_membership().expect("the only log entry for initializing has to be membership log");
        self.check_members_contain_me(m)?;

        let log_id = entry.get_log_id();
        tracing::debug!("update effective membership: log_id:{} {}", log_id, m.summary());

        let em = EffectiveMembership::new_arc(Some(*log_id), m.clone());
        self.state.membership_state.append(em.clone());

        self.output.push_command(Command::UpdateMembership { membership: em });

        self.server_state_handler().update_server_state_if_changed();

        self.output.push_command(Command::MoveInputCursorBy { n: l });

        // With the new config, start to elect to become leader
        self.elect();

        Ok(())
    }

    /// Start to elect this node as leader
    #[tracing::instrument(level = "debug", skip(self))]
    pub(crate) fn elect(&mut self) {
        let v = Vote::new(self.state.vote_ref().leader_id().term + 1, self.config.id);
        // Safe unwrap(): it won't reject itself ˙–˙
        self.vote_handler().handle_message_vote(&v).unwrap();

        // Safe unwrap()
        let leader = self.internal_server_state.leading_mut().unwrap();
        leader.grant_vote_by(self.config.id);
        let quorum_granted = leader.is_vote_granted();

        // Fast-path: if there is only one node in the cluster.

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
        tx: Option<RaftRespTx<T, E>>,
    ) -> Option<(LeaderHandler<NID, N>, Option<RaftRespTx<T, E>>)>
    where
        E: From<ForwardToLeader<NID, N>>,
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
    pub(crate) fn handle_vote_req(&mut self, req: VoteRequest<NID>) -> VoteResponse<NID> {
        let now = *self.timer.now();
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

        let res = self.vote_handler().handle_message_vote(&req.vote);

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
    pub(crate) fn handle_vote_resp(&mut self, target: NID, resp: VoteResponse<NID>) {
        tracing::debug!(
            resp = display(resp.summary()),
            target = display(target),
            "handle_vote_resp"
        );
        tracing::debug!(
            my_vote = display(self.state.vote_ref()),
            my_last_log_id = display(self.state.last_log_id().summary()),
            "handle_vote_resp"
        );

        // If this node is no longer a leader(i.e., electing), just ignore the delayed vote_resp.
        let leader = match &mut self.internal_server_state {
            InternalServerState::Leading(l) => l,
            InternalServerState::Following => return,
        };

        if &resp.vote < self.state.vote_ref() {
            debug_assert!(!resp.vote_granted);
        }

        if resp.vote_granted {
            leader.grant_vote_by(target);

            let quorum_granted = leader.is_vote_granted();
            if quorum_granted {
                tracing::debug!("quorum granted vote");
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
        let _ = self.vote_handler().handle_message_vote(&resp.vote);

        // Seen a higher log. Record it so that the next election will be delayed for a while.
        if resp.last_log_id.as_ref() > self.state.last_log_id() {
            self.set_greater_log();
        }
    }

    /// Append entries to follower/learner.
    ///
    /// Also clean conflicting entries and update membership state.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn handle_append_entries_req<'a, Ent>(
        &mut self,
        vote: &Vote<NID>,
        prev_log_id: Option<LogId<NID>>,
        entries: &[Ent],
        leader_committed: Option<LogId<NID>>,
    ) -> AppendEntriesResponse<NID>
    where
        Ent: RaftEntry<NID, N> + 'a,
    {
        tracing::debug!(
            vote = display(vote),
            prev_log_id = display(prev_log_id.summary()),
            entries = display(DisplaySlice(entries)),
            leader_committed = display(leader_committed.summary()),
            "append-entries request"
        );
        tracing::debug!(
            my_vote = display(self.state.vote_ref()),
            my_last_log_id = display(self.state.last_log_id().summary()),
            my_committed = display(self.state.committed().summary()),
            "local state"
        );

        let res = self.vote_handler().handle_message_vote(vote);
        if let Err(rejected) = res {
            return rejected.into();
        }

        // Vote is legal.

        let mut fh = self.following_handler();
        fh.append_entries(prev_log_id, entries, leader_committed)
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
            if !em.is_voter(&self.config.id) && self.state.is_leading(&self.config.id) {
                tracing::debug!("leader {} is stepping down", self.config.id);
                self.vote_handler().become_following();
            }
        }
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn finish_building_snapshot(&mut self, meta: SnapshotMeta<NID, N>) {
        tracing::info!("finish_building_snapshot: {:?}", meta);

        let mut h = self.snapshot_handler();

        let updated = h.update_snapshot(meta);
        if !updated {
            return;
        }

        self.log_handler().update_purge_upto();

        if self.internal_server_state.is_leading() {
            // If it is leading, it must not delete a log that is in use by a replication task.
            self.replication_handler().try_purge_log();
        } else {
            // For follower/learner, no other tasks are using logs, just purge.
            self.log_handler().purge_log();
        }
    }
}

/// Supporting util
impl<NID, N> Engine<NID, N>
where
    N: Node,
    NID: NodeId,
{
    /// Vote is granted by a quorum, leader established.
    #[tracing::instrument(level = "debug", skip_all)]
    fn establish_leader(&mut self) {
        self.vote_handler().commit_vote();

        let mut rh = self.replication_handler();

        // It has to setup replication stream first because append_blank_log() may update the
        // committed-log-id(a single leader with several learners), in which case the
        // committed-log-id will be at once submitted to replicate before replication stream
        // is built. TODO: But replication streams should be built when a node enters
        // leading state.       Thus append_blank_log() can be moved before
        // rebuild_replication_streams()

        rh.rebuild_replication_streams();
        rh.append_blank_log();
        rh.initiate_replication(SendNone::False);
    }

    /// Check if a raft node is in a state that allows to initialize.
    ///
    /// It is allowed to initialize only when `last_log_id.is_none()` and `vote==(term=0,
    /// node_id=0)`. See: [Conditions for initialization](https://datafuselabs.github.io/openraft/cluster-formation.html#conditions-for-initialization)
    fn check_initialize(&self) -> Result<(), NotAllowed<NID>> {
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
    fn check_members_contain_me(&self, m: &Membership<NID, N>) -> Result<(), NotInMembers<NID, N>> {
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

    pub(crate) fn vote_handler(&mut self) -> VoteHandler<NID, N> {
        VoteHandler {
            config: &self.config,
            state: &mut self.state,
            timer: &mut self.timer,
            output: &mut self.output,
            internal_server_state: &mut self.internal_server_state,
        }
    }

    pub(crate) fn log_handler(&mut self) -> LogHandler<NID, N> {
        LogHandler {
            config: &mut self.config,
            state: &mut self.state,
            output: &mut self.output,
        }
    }

    pub(crate) fn snapshot_handler(&mut self) -> SnapshotHandler<NID, N> {
        SnapshotHandler {
            state: &mut self.state,
            output: &mut self.output,
        }
    }

    pub(crate) fn leader_handler(&mut self) -> Result<LeaderHandler<NID, N>, ForwardToLeader<NID, N>> {
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

    pub(crate) fn replication_handler(&mut self) -> ReplicationHandler<NID, N> {
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

    pub(crate) fn following_handler(&mut self) -> FollowingHandler<NID, N> {
        debug_assert!(self.internal_server_state.is_following());

        FollowingHandler {
            config: &mut self.config,
            state: &mut self.state,
            output: &mut self.output,
        }
    }

    pub(crate) fn server_state_handler(&mut self) -> ServerStateHandler<NID, N> {
        ServerStateHandler {
            config: &self.config,
            state: &mut self.state,
            output: &mut self.output,
        }
    }
}
