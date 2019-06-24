//! A module encapsulating the core `Raft` actor and its logic.

use std::{
    collections::BTreeMap,
    time::Duration,
};

use actix::prelude::*;
use failure::Fail;
use log::{error, warn};

use crate::{
    config::Config,
    proto,
    storage::{
        self, StorageError, StorageResult, AppendLogEntries, AppendLogEntriesData,
        ApplyEntriesToStateMachine, GetLogEntries, InitialState, SaveHardState,
        HardState, RaftStorage,
    },
};

/// An error message for communication errors while attempting to append log entries.
const COMM_ERR_APPEND_LOG_ENTRIES: &str = "Error communicating with storage actor for appending log entries.";

/// An error message for communication errors while attempting to fetch log entries.
const COMM_ERR_FETCH_LOG_ENTRIES: &str = "Error communicating with storage actor for fetching log entries.";

/// An error message for communication errors while attempting to save hard state.
const COMM_SAVE_HARD_STATE: &str = "Error communicating with storage actor for saving hard state.";

/// A Raft cluster node's ID.
pub type NodeId = u64;

/// The state of the Raft node.
#[derive(Eq, PartialEq)]
pub enum NodeState {
    /// A non-standard Raft state indicating that the node is initializing.
    Initializing,
    /// A non-standard Raft state indicating that the node is awaiting an admin command to begin.
    ///
    /// The Raft node will only be in this state when it comes online for the very first time
    /// without any state recovered from disk. In such a state, the parent application may have
    /// this new node added to an already running cluster, may have the node start as the leader
    /// of a new standalone cluster, or have the node initialize with a specific config.
    ///
    /// This state gives control over Raft's initial cluster formation and node startup to the
    /// application which is using this system.
    Standby,
    /// The node is actively replicating logs from the leader.
    ///
    /// The node is passive when it is in this state. It issues no requests on its own but simply
    /// responds to requests from leaders and candidates.
    Follower,
    /// The node has detected an election timeout so is requesting votes to become leader.
    Candidate,
    /// The node is actively functioning as the Raft cluster leader.
    ///
    /// The leader handles all client requests. If a client contacts a follower, the follower must
    /// redirects it to the leader.
    Leader(LeaderState),
}

/// Volatile state specific to the Raft leader.
///
/// This state is reinitialized after an election.
#[derive(Eq, PartialEq)]
pub struct LeaderState {
    /// A mapping of node IDs to the index of the next log entry to send.
    ///
    /// Each entry is initialized to leader's last log index + 1. Per the Raft protocol spec,
    /// this value may be decremented as new nodes enter the cluster and need to catch-up.
    ///
    /// When a leader first comes to power, it initializes all `next_index` values to the index
    /// just after the last one in its log.
    ///
    /// If a follower’s log is inconsistent with the leader’s, the AppendEntries consistency check
    /// will fail in the next AppendEntries RPC. After a rejection, the leader decrements
    /// `next_index` and retries the AppendEntries RPC. Eventually `next_index` will reach a point
    /// where the leader and follower logs match. When this happens, AppendEntries will succeed,
    /// which removes any conflicting entries in the follower’s log and appends entries from the
    /// leader’s log (if any). Once AppendEntries succeeds, the follower’s log is consistent with
    /// the leader’s, and it will remain that way for the rest of the term.
    next_index: BTreeMap<NodeId, u64>,
    /// A mapping of node IDs to the highest log entry index known to be replicated thereof.
    ///
    /// Each entry is initialized to 0, and increases per the Raft data replication protocol.
    match_index: BTreeMap<NodeId, u64>,
}

/// An actor which implements the Raft protocol's core business logic.
///
/// For more information on the Raft protocol, see the specification here:
/// https://raft.github.io/raft.pdf (**pdf warning**).
///
/// The beginning of §5, the spec has a condensed summary of the Raft consensus algorithm. This
/// crate, and especially this actor, attempts to follow the terminology and nomenclature used
/// there as precisely as possible to aid in understanding this system.
///
/// ### api
/// This actor's API is broken up into 4 different layers, all based on message handling. In order
/// to effectively use this actor, only these 4 interfaces need to considered.
///
/// #### raft request messages
/// These are Raft request PRCs coming from other nodes of the cluster. This interface is
/// implemented as an `actix::Handler`, so a future will be returned which will resolve with the
/// appropriate response type.
///
/// Typically your application's networking layer will decode these message types and simply pass
/// them over to this handler, await the response, and then send the response back over the wire
/// to the caller. However, this is entirely application specific, so at the end of the day, all
/// one needs to do is send the message to this actor, await the response, and then handle the
/// response as needed by your application.
///
/// #### client request messages
/// These are messages coming from your application's clients. This interface is implemented as an
/// `actix::Handler`, so a future will be returned which will resolve with the appropriate
/// response type. Only data mutating messages should ever need to go through Raft. The contents
/// of these messages are entirely specific to your application.
///
/// #### outbound raft request
/// These are messages originating from the Raft actor which are destined for some peer node of
/// the Raft cluster. When the Raft actor is instantiated, an `actix::Recipient` must be supplied
/// which is expected to handle the networking layer logic of actually sending the message to the
/// target peer.
///
/// The networking layer is application specific, so no constraints are put in place in terms of
/// how your application's nodes are to communicate with each other. The only thing that is
/// required is that the message be sent and the response be delivered back.
///
/// Per the Raft spec, message delivery failure from a leader to a follower is to be retried
/// indefinitely, so that is how this actor is implemented.
///
/// #### storage
/// The storage interface is typically going to be the most involved as this is where your
/// application really exists. SQL, NoSQL, mutable, immutable, KV, append only ... whatever your
/// application's data model, this is where it comes to life.
///
/// The storage interface is provided as an `actix::Addr<S: RaftStorage>`. The generic type `S`
/// must implement the `RaftStorage` trait, which is composed of a series of actix message
/// handling traits.
///
/// Depending on the data storage system being used, the actor my be sync or async. It just needs
/// to implement handlers for the needed actix message types.
///
/// Note that currently, when this actor encounters an error from the storage layer, it will stop.
/// The rest of the system may remain online as long as is needed, but this actor will stop in
/// order to avoid data corruption or other such issues.
pub struct Raft<S: RaftStorage> {
    /// This node's ID.
    id: NodeId,
    /// This node's runtime config.
    config: Config,
    /// All currently known members of the Raft cluster.
    members: Vec<NodeId>,
    /// The current state of this Raft node.
    state: NodeState,
    /// An output channel for sending Raft request messages to peers.
    out: actix::Recipient<RaftRequest>,
    /// The address of the actor responsible for implementing the `RaftStorage` interface.
    storage: actix::Addr<S>,

    /// The index of the highest log entry known to be committed cluster-wide.
    ///
    /// The definition of a committed log is that the leader which has created the log has
    /// successfully replicated the log to a majority of the cluster. This value is only ever
    /// updated by way of an AppendEntries RPC from the leader.
    ///
    /// Is initialized to 0, and increases monotonically. This is always based on the leader's
    /// commit index which is communicated to other members via the AppendEntries protocol.
    commit_index: u64,
    /// The index of the highest log entry which has been applied to the local state machine.
    ///
    /// Is initialized to 0, increases monotonically following the `commit_index` as logs are
    /// applied to the state machine (via the storage interface).
    last_applied: u64,
    /// The current term.
    ///
    /// Is initialized to 0 on first boot, and increases monotonically. This is normally based on
    /// the leader's term which is communicated to other members via the AppendEntries protocol,
    /// but this may also be incremented when a follower becomes a candidate.
    current_term: u64,
    /// The ID of the candidate which received this node's vote for the current term.
    ///
    /// Each server will vote for at most one candidate in a given term, on a
    /// first-come-first-served basis. See §5.4.1 for additional restriction on votes.
    voted_for: Option<NodeId>,

    /// The index of the last log to be appended.
    last_log_index: u64,
    /// The term of the last log to be appended.
    last_log_term: u64,

    /// A flag to indicate if this system is currently appending logs.
    is_appending_logs: bool,
    /// A flag to indicate if this system is currently applying logs to the state machine.
    is_applying_logs_to_state_machine: bool,

    /// A handle to the election timeout callback.
    election_timeout: Option<actix::SpawnHandle>,
}

impl<S> Raft<S> where S: RaftStorage {
    /// Create a new Raft instance.
    ///
    /// This actor will need to be started after instantiation, which must be done within a
    /// running actix system.
    ///
    /// TODO: add an example on how to create and start an instance.
    pub fn new(id: NodeId, config: Config, out: actix::Recipient<RaftRequest>, storage: Addr<S>) -> Self {
        let state = NodeState::Initializing;
        Self{
            id, config, members: vec![id], state, out, storage,
            commit_index: 0, last_applied: 0,
            current_term: 0, voted_for: None,
            last_log_index: 0, last_log_term: 0,
            is_appending_logs: false, is_applying_logs_to_state_machine: false,
            election_timeout: None,
        }
    }

    /// Append the given entries to the log.
    ///
    /// This routine also encapsulates all logic which must be performed related to appending log
    /// entries.
    ///
    /// One important piece of logic to note here is the handling of config change entries. Per
    /// the Raft spec in §6:
    ///
    /// > Once a given server adds the new configuration entry to its log, it uses that
    /// > configuration for all future decisions (a server always uses the latest configuration in
    /// > its log, regardless of whether the entry is committed).
    ///
    /// This routine will extract the most recent (the latter most) entry in the given payload of
    /// entries which is a config change entry and will update the node's member state based on
    /// that entry.
    fn append_log_entries(
        &mut self, ctx: &mut Context<Self>, entries: Vec<proto::Entry>,
    ) -> impl ActorFuture<Actor=Self, Item=AppendLogEntriesData, Error=RaftError> {
        // Check the given entries for any config changes and take the most recent.
        use proto::entry::EntryType;
        let last_conf_change = entries.iter().filter_map(|ent| match &ent.entry_type {
            Some(EntryType::ConfigChange(conf)) => Some(conf),
            _ => None,
        }).last();
        if let Some(conf) = last_conf_change {
            // Update membership info & apply hard state.
            self.members = conf.members.clone();
            self.save_hard_state(ctx);
        }

        fut::wrap_future(self.storage.send(AppendLogEntries(entries)))
            .map_err(|err, _: &mut Self, _| Self::map_messaging_error(err))
            .and_then(|res, act, ctx| fut::FutureResult::from(act.map_storage_result(ctx, res)))
    }

    /// Begin the process of applying logs to the state machine.
    fn apply_logs_to_state_machine(&mut self, ctx: &mut Context<Self>) {
        // Fetch the series of entries which must be applied to the state machine.
        self.is_applying_logs_to_state_machine = true;
        let f = fut::wrap_future(self.storage.send(GetLogEntries{start: self.last_applied, stop: self.commit_index + 1}))
            .map_err(|err, _: &mut Self, _| Self::map_messaging_error(err))
            .and_then(|res, act, ctx| fut::FutureResult::from(act.map_storage_result(ctx, res)))

            // Send the entries over to the storage engine to be applied to the state machine.
            .and_then(|entries, act, _| {
                fut::wrap_future(act.storage.send(ApplyEntriesToStateMachine(entries)))
                    .map_err(|err, _: &mut Self, _| Self::map_messaging_error(err))
                    .and_then(|res, act, ctx| fut::FutureResult::from(act.map_storage_result(ctx, res)))
            })

            // Update self to reflect progress on applying logs to the state machine.
            .and_then(|data, act, _| {
                act.last_applied = data.index;
                act.is_applying_logs_to_state_machine = false;
                fut::FutureResult::from(Ok(()))
            })

            // Log any errors which may have come from the process of applying log entries.
            .map_err(|err, _, _| {
                error!("{}", err)
            });
        let _ = ctx.spawn(f);
    }

    /// Handle requests from Raft leader to append log entries.
    ///
    /// This method implements the append entries algorithm and upholds all of the safety checks
    /// detailed in §5.3.
    ///
    /// The essential goal of this algorithm is that the receiver (the node on which this method
    /// is being executed) must find the exact entry in its log specified by the RPC's last index
    /// and last term fields, and then begin writing the new entries thereafter.
    ///
    /// When the receiver can not find the entry specified in the RPC's prev index & prev term
    /// fields, it will respond with a failure to the leader. **This implementation of Raft
    /// includes the _conflicting term_ optimization** which is intended to reduce the number of
    /// rejected append entries RPCs from followers which are lagging behind, which is detailed in
    /// §5.3. In such cases, if the Raft cluster is configured with a snapshot policy other than
    /// `Disabled`, the leader will make a determination if an `InstallSnapshot` RPC should be
    /// sent to this node.
    ///
    /// In Raft, the leader handles inconsistencies by forcing the followers’ logs to duplicate
    /// its own. This means that conflicting entries in follower logs will be overwritten with
    /// entries from the leader’s log. §5.4 details the safety of this protocol. It is important
    /// to note that logs which are _committed_ will not be overwritten. This is a critical
    /// feature of Raft.
    ///
    /// Raft also gurantees that only logs which have been comitted may be applied to the state
    /// machine, which ensures that there will never be a case where a log needs to be reverted
    /// after being applied to the state machine.
    ///
    /// #### inconsistency example
    /// Followers may receive valid append entries requests from leaders, append them, respond,
    /// and before the leader is able to replicate the entries to a majority of nodes, the leader
    /// may die, a new leader may be elected which does not have the same entries, as they were
    /// not replicated to a majority of followers, and the new leader will proceeed to overwrite
    /// the inconsistent entries.
    fn handle_append_entries_request(
        &mut self, ctx: &mut Context<Self>, msg: proto::AppendEntriesRequest,
    ) -> impl ActorFuture<Actor=Self, Item=proto::AppendEntriesResponse, Error=RaftError> {
        // Don't interact with non-cluster members.
        if !self.members.contains(&msg.leader_id) {
            return fut::Either::A(
                fut::Either::A(fut::err(RaftError::RPCFromUnknownNode))
            );
        }

        // If message's term is less than most recent term, then we do not honor the request.
        if &msg.term < &self.current_term {
            return fut::Either::A(
                fut::Either::A(fut::ok(proto::AppendEntriesResponse{term: self.current_term, success: false, conflict_opt: None}))
            );
        }

        // Update current term as needed & ensure we are in the follower state if needed.
        if &msg.term > &self.current_term {
            self.update_current_term(ctx, msg.term);
        }

        // Kick off process of applying logs to state machine based on `msg.leader_commit`.
        self.commit_index = msg.leader_commit; // The value for `self.commit_index` is only updated here.
        if &self.commit_index > &self.last_applied && !self.is_applying_logs_to_state_machine {
            self.apply_logs_to_state_machine(ctx);
        }

        // If the AppendEntries RPC has no entries, then this was just a heartbeat.
        if msg.entries.len() == 0 {
            return fut::Either::A(
                fut::Either::A(fut::ok(proto::AppendEntriesResponse{term: self.current_term, success: true, conflict_opt: None}))
            );
        }

        // If logs are already being appended, then just abort.
        if self.is_appending_logs {
            return fut::Either::A(
                fut::Either::A(fut::ok(proto::AppendEntriesResponse{term: self.current_term, success: false, conflict_opt: None}))
            );
        }

        // If RPC's `prev_log_index` is 0, or the RPC's previous log info matches the local
        // previous log info, then replication is g2g.
        let term = self.current_term;
        self.is_appending_logs = true;
        if &msg.prev_log_index == &u64::min_value() || (&msg.prev_log_index == &self.last_log_index && &msg.prev_log_term == &self.last_log_term) {
            return fut::Either::A(fut::Either::B(
                self.append_log_entries(ctx, msg.entries)
                    .map(move |res, act, _| {
                        act.last_log_index = res.index;
                        act.last_log_term = res.term;
                        proto::AppendEntriesResponse{term, success: true, conflict_opt: None}
                    })
                    .then(|res, act, _| {
                        act.is_appending_logs = false;
                        fut::FutureResult::from(res)
                    })));
        }

        // Previous log info doesn't immediately line up, so perform log consistency check and
        // proceed based on its result.
        fut::Either::B(self.log_consistency_check(ctx, msg.prev_log_index, msg.prev_log_term)
            .and_then(move |res, act, ctx| match res {
                Some(conflict_opt) => fut::Either::A(fut::FutureResult::from(Ok(
                    proto::AppendEntriesResponse{term, success: false, conflict_opt: Some(conflict_opt)}
                ))),
                None => fut::Either::B(act.append_log_entries(ctx, msg.entries)
                    // At this point, we have successfully appended the new entries to the log.
                    // We now need to update our `last_log_*` info and build a response.
                    .and_then(move |res, act, _| {
                        act.last_log_index = res.index;
                        act.last_log_term = res.term;
                        fut::FutureResult::from(Ok(proto::AppendEntriesResponse{term, success: true, conflict_opt: None}))
                    })),
            })
            .then(|res, act, _| {
                act.is_appending_logs = false;
                fut::FutureResult::from(res)
            }))
    }

    /// Handle requests from peers to cast a vote for a new leader.
    fn handle_vote_request(&mut self, ctx: &mut Context<Self>, msg: proto::VoteRequest) -> Result<proto::VoteResponse, RaftError> {
        // Don't interact with non-cluster members.
        if !self.members.contains(&msg.candidate_id) {
            return Err(RaftError::RPCFromUnknownNode);
        }

        // If candidate's current term is less than this nodes current term, reject.
        if &msg.term < &self.current_term {
            return Ok(proto::VoteResponse{term: self.current_term, vote_granted: false});
        }

        // If candidate's log is not at least as up-to-date as this node, then reject.
        if &msg.last_log_term < &self.last_log_term || &msg.last_log_index < &self.last_log_index {
            return Ok(proto::VoteResponse{term: self.current_term, vote_granted: false});
        }

        // Candidate's log is up-to-date so handle voting conditions.

        // If term is newer than current term, cast vote.
        if &msg.term > &self.current_term {
            self.voted_for = Some(msg.candidate_id);
            self.update_current_term(ctx, msg.term);
            return Ok(proto::VoteResponse{term: self.current_term, vote_granted: true});
        }

        // Term is the same as current term. This will be rare, but could come about from some error conditions.
        match &self.voted_for {
            // This node has already voted for the candidate.
            Some(candidate_id) if candidate_id == &msg.candidate_id => {
                Ok(proto::VoteResponse{term: self.current_term, vote_granted: true})
            }
            // This node has already voted for a different candidate.
            Some(_) => Ok(proto::VoteResponse{term: self.current_term, vote_granted: false}),
            // This node has not already voted, so vote for the candidate.
            None => {
                self.voted_for = Some(msg.candidate_id);
                self.save_hard_state(ctx);
                Ok(proto::VoteResponse{term: self.current_term, vote_granted: true})
            },
        }
    }

    /// Perform the initialization routine for the Raft node.
    ///
    /// If this node has configuration present from being online previously, then this node will
    /// begin a standard lifecycle as a follower. If this node is pristine, then it will wait in
    /// standby mode.
    ///
    /// ### previous state | follower
    /// If the node has previous state, then there are a few cases to account for.
    ///
    /// If the node has been offline for some time and was removed from the cluster, no problem.
    /// Any RPCs sent from this node will be rejected until it is added to the cluster. Once it is
    /// added to the cluster again, the standard Raft protocol will resume as normal.
    ///
    /// If the node went down only very briefly, then it should immediately start receiving
    /// heartbeats and resume as normal, else it will start an election if it doesn't receive any
    /// heartbeats from the leader per normal Raft protocol.
    ///
    /// If the node was running standalone, it will win the election and resume as a standalone.
    ///
    /// ### pristine state | standby
    /// While in standby mode, the Raft leader of the current cluster may discover this node and
    /// add it to the cluster. In such a case, it will begin receiving heartbeats from the leader
    /// and business proceeds as usual.
    ///
    /// If there is no current cluster, while in standby mode, the node may receive an admin
    /// command instructing it to campaign with a specific config, or to begin operating as the
    /// leader of a standalone cluster.
    fn initialize(&mut self, ctx: &mut Context<Self>, state: InitialState) {
        self.last_log_index = state.last_log_index;
        self.last_log_term = state.last_log_term;
        self.current_term = state.hard_state.current_term;
        self.voted_for = state.hard_state.voted_for;
        self.members = state.hard_state.members;
        self.last_applied = state.last_applied_log;

        // Set initial state based on state recovered from disk.
        let is_only_configured_member = self.members.len() == 1 && self.members.contains(&self.id);
        if is_only_configured_member || &self.last_log_index != &u64::min_value() {
            self.state = NodeState::Follower;
            self.update_election_timeout(ctx);
        } else {
            self.state = NodeState::Standby;
        }
    }

    /// Perform the AppendEntries RPC consistency check.
    ///
    /// If the log entry at the specified index does not exist, the most recent entry in the log
    /// will be used to build and return a `ConflictOpt` struct to be sent back to the leader.
    ///
    /// If The log entry at the specified index does exist, but the terms to no match up, this
    /// implementation will fetch the last 50 entries from the given index, and will use the
    /// earliest entry from the log which is still in the given term to build a `ConflictOpt`
    /// struct to be sent back to the leader.
    ///
    /// If everyhing checks out, a `None` value will be returned and log replication may continue.
    fn log_consistency_check(
        &mut self, _: &mut Context<Self>, index: u64, term: u64,
    ) -> impl ActorFuture<Actor=Self, Item=Option<proto::ConflictOpt>, Error=RaftError> {
        let storage = self.storage.clone();
        fut::wrap_future(self.storage.send(GetLogEntries{start: index, stop: index}))
            .map_err(|err, _: &mut Self, _| Self::map_messaging_error(err))
            .and_then(|res, act, ctx| fut::FutureResult::from(act.map_storage_result(ctx, res)))
            .and_then(move |res, act, _| {
                match res.last() {
                    // The target entry was not found. This can only mean that we don't have the
                    // specified index yet. Use the last known index & term.
                    None => fut::Either::A(fut::FutureResult::from(Ok(Some(proto::ConflictOpt{
                        term: act.last_log_term,
                        index: act.last_log_index,
                    })))),
                    // The target entry was found. Compare its term with target term to ensure
                    // everything is consistent.
                    Some(entry) => {
                        let entry_term = entry.term;
                        if entry_term == term {
                            // Everything checks out. We're g2g.
                            fut::Either::A(fut::FutureResult::from(Ok(None)))
                        } else {
                            // Logs are inconsistent. Fetch the last 50 logs, and use the last
                            // entry of that payload which is still in the target term for
                            // conflict optimization.
                            fut::Either::B(fut::wrap_future(storage.send(GetLogEntries{start: index, stop: index}))
                                .map_err(|err, _: &mut Self, _| Self::map_messaging_error(err))
                                .and_then(|res, act, ctx| fut::FutureResult::from(act.map_storage_result(ctx, res)))
                                .and_then(move |res, _, _| {
                                    match res.into_iter().filter(|entry| entry.term == term).nth(0) {
                                        Some(entry) => fut::FutureResult::from(Ok(Some(proto::ConflictOpt{
                                            term: entry.term,
                                            index: entry.index,
                                        }))),
                                        None => fut::FutureResult::from(Ok(Some(proto::ConflictOpt{
                                            term: entry_term,
                                            index: index,
                                        }))),
                                    }
                                }))
                        }
                    }
                }
            })
    }

    /// A simple mapping function to log and transform an `actix::MailboxError`.
    fn map_messaging_error(err: actix::MailboxError) -> RaftError {
        error!("An internal actix messaging error was encountered. {}", err);
        RaftError::InternalMessagingError
    }

    /// A simple mapping function to transform a `StorageResult` from the storage layer.
    ///
    /// NOTE WELL: currently, this routine will also stop the Raft node, as storage errors are
    /// seen as non-recoverable.
    fn map_storage_result<T>(&mut self, ctx: &mut Context<Self>, res: StorageResult<T>) -> Result<T, RaftError> {
        res.map_err(|err| {
            error!("Storage error encountered. Stopping Raft node.");
            ctx.stop();
            RaftError::StorageError(err)
        })
    }

    /// Save the Raft node's current hard state to disk.
    fn save_hard_state(&mut self, ctx: &mut Context<Self>) {
        let hs = HardState{current_term: self.current_term, voted_for: self.voted_for, members: self.members.clone()};
        let f = fut::wrap_future(self.storage.send(SaveHardState(hs)))
            .map_err(|err, _: &mut Self, _| Self::map_messaging_error(err))
            .and_then(|res, act, ctx| fut::FutureResult::from(act.map_storage_result(ctx, res)))
            .map_err(|err, _, _| {
                error!("{}", err)
            });

        ctx.spawn(f);
    }

    /// Update the node's current term.
    ///
    /// This routine will also perform any other logic which must take place as part of updating
    /// the current term.
    fn update_current_term(&mut self, ctx: &mut Context<Self>, term: u64) {
        // Ensure this isn't a no-op.
        if &self.current_term == &term {
            return;
        }

        // Update current term and update node state if needed.
        if let &NodeState::Leader(_) = &self.state {
            self.update_election_timeout(ctx);
        }
        self.state = NodeState::Follower;
        self.current_term = term;
        self.save_hard_state(ctx);
    }

    /// Update the election timeout process.
    ///
    /// This will run the nodes election timeout mechanism to ensure that elections are held if
    /// too much time passes before hearing from a leader or a candidate.
    ///
    /// The election timeout will be updated everytime this node receives an RPC from the leader
    /// as well as any time a candidate node sends a RequestVote RPC. We reset on candidate RPCs
    /// iff the RPC is a valid vote request.
    fn update_election_timeout(&mut self, ctx: &mut Context<Self>) {
        // Cancel any current election timeout before spawning a new one.
        if let Some(handle) = self.election_timeout.take() {
            ctx.cancel_future(handle);
        }
        let timeout = Duration::from_millis(self.config.election_timeout_millis);
        self.election_timeout = Some(ctx.run_later(timeout, |act, ctx| act.start_campaign(ctx)));
    }

    /// Begin a new election campaign for this node. See §5.2.
    ///
    /// As part of an election campaign, a follower increments its current term and transitions to
    /// candidate state, it then votes for itself (will then save its hard state) and issues
    /// RequestVote RPCs in parallel to each of the other nodes in the cluster.
    ///
    /// A candidate remains in the candidate state until one of three things happens:
    ///
    /// 1. It wins the election.
    /// 2. Another server establishes itself as leader.
    /// 3. A period of time goes by with no winner.
    ///
    /// (1) a candidate wins an election if it receives votes from a majority of the servers
    /// in the full cluster for the same term. Each server will vote for at most one candidate in
    /// a given term, on a first-come-first-served basis (§5.4 adds an additional restriction on
    /// votes). The majority rule ensures that at most one candidate can win the election for a
    /// particular term. Once a candidate wins an election, it becomes leader. It then sends
    /// heartbeat messages to all of the other servers to establish its authority and prevent new
    /// elections.
    ///
    /// (2) While waiting for votes, a candidate may receive an AppendEntries RPC from another
    /// server claiming to be leader. If the leader’s term in the RPC is at least as large as the
    /// candidate’s current term, then the candidate recognizes the leader as legitimate and
    /// returns to follower state. If the term in the RPC is smaller than the candidate’s current
    /// term, then the candidate rejects the RPC and continues in candidate state.
    ///
    /// (3) The third possible outcome is that a candidate neither wins nor loses the election: if
    /// many followers become candidates at the same time, votes could be split so that no
    /// candidate obtains a majority. When this happens, each candidate will time out and start a
    /// new election by incrementing its term and initiating another round of RequestVote RPCs.
    /// The randomization of election timeouts per node helps to avoid this issue.
    fn start_campaign(&mut self, ctx: &mut Context<Self>) {
        self.state = NodeState::Candidate;
        self.current_term += 1;
        self.voted_for = Some(self.id);
        self.save_hard_state(ctx);

        // Send RPCs to all members in parallel.
        // <<<<<<<< RESUME HERE IMMEDIATE
    }
}

impl<S: RaftStorage> Actor for Raft<S> {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        // Fetch the node's initial state from the storage actor & initialize.
        let f = fut::wrap_future(self.storage.send(storage::GetInitialState))
            .map_err(|err, _: &mut Self, _| Self::map_messaging_error(err))
            .and_then(|res, act, ctx| fut::FutureResult::from(act.map_storage_result(ctx, res)))
            .map_err(|_, _, _| ())
            .map(|state, act, ctx| act.initialize(ctx, state));

        ctx.spawn(f);
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// RaftRequest ///////////////////////////////////////////////////////////////////////////////////

/// An actix::Message wrapping a protobuf RaftRequest.
pub struct RaftRequest(pub proto::RaftRequest);

impl Message for RaftRequest {
    type Result = Result<proto::RaftResponse, RaftError>;
}

impl<S: RaftStorage> Handler<RaftRequest> for Raft<S> {
    type Result = ResponseActFuture<Self, proto::RaftResponse, RaftError>;

    /// Handle inbound Raft request messages.
    ///
    /// Typically the Raft messages sent to this handler will have come from peer nodes of the
    /// Raft cluster, usuall via your application's networking layer. This handler guarantees that
    /// the appropriate Raft response message type will be returned in response according to the
    /// Raft spec.
    ///
    /// ### errors
    /// It is rare that an error will be returned from this handler. The most typical error
    /// conditions are related to storage failures; however that is not the only error case. In
    /// the case of a storage failure however, this actor will go into the stopping state and shut
    /// down.
    ///
    /// On an application level, it would be prudent to fail the associated request when an error
    /// takes place here and have the caller perform a retry.
    fn handle(&mut self, msg: RaftRequest, ctx: &mut Self::Context) -> Self::Result {
        // Only handle requests if actor has finished initialization.
        if self.state == NodeState::Initializing {
            warn!("Received RaftRequest before initialization was complete.");
            return Box::new(fut::err(RaftError::Initializing));
        }

        // Unpack the given message and pass to the appropriate handler.
        use proto::raft_request::Payload;
        use proto::raft_response::Payload as ResponsePayload;
        match msg.0.payload {
            Some(Payload::AppendEntries(payload)) => {
                Box::new(self.handle_append_entries_request(ctx, payload)
                    .map(|res, _, _| proto::RaftResponse{
                        payload: Some(ResponsePayload::AppendEntries(res)),
                    }))
            },
            Some(Payload::Vote(payload)) => {
                Box::new(fut::FutureResult::from(self.handle_vote_request(ctx, payload))
                    .map(|res, _, _| proto::RaftResponse{
                        payload: Some(ResponsePayload::Vote(res)),
                    }))
            },
            Some(Payload::InstallSnapshot(_payload)) => {
                // TODO: finish this up.
                Box::new(fut::err(RaftError::Initializing))
            },
            None => {
                warn!("RaftRequest received which had an empty or unknown payload.");
                Box::new(fut::err(RaftError::UnknownRequestReceived))
            }
        }
    }
}

/// Error variants which may come from the Raft actor.
#[derive(Debug, Fail)]
pub enum RaftError {
    /// The Raft node is still initializing and couldn't handle the request.
    #[fail(display="Raft node is still initializing.")]
    Initializing,
    /// The Raft node encountered an internal messaging error.
    ///
    /// This type of error is related to the underlying actix framework and will be quite rare to
    /// come by. **This should be considered a transient type of error.**
    #[fail(display="Raft node encountered an internal messaging error.")]
    InternalMessagingError,
    /// An RPC was received from a node which does not appear to be a member of the cluster.
    #[fail(display="Raft node received an RPC frame from an unknown node.")]
    RPCFromUnknownNode,
    /// An error coming from the storage layer.
    #[fail(display="{}", _0)]
    StorageError(StorageError),
    /// The Raft node received an unknown request type.
    #[fail(display="Raft node received an unknown request type.")]
    UnknownRequestReceived,
}

// TODO:
// ### admin commands
// - get AdminCommands setup and implemented.
//
// ### elections
// - Finish up election system.
//
// ### clients
// - create client message protobuf, used to generically wrap any type of client request.
//   Put together docs on how applications should mitigate client retries which would
//   lead to duplicates (request serial number tracking).
// - implement handler for client requests.
//
// ### observability
// - ensure that internal state transitions and updates are emitted for host application use. Such
// as NodeState changes, membershipt changes, errors from async ops.
//
// ### testing
// - setup testing framework to assert accurate behavior of Raft implementation and adherence to
// Raft's safety protocols.
// - all actor based. Transport layer can be a simple message passing mechanism.
// - will probably need to implement MemoryStroage for testing.
