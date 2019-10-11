//! Replication stream actor.
//!
//! This module encapsulates the `ReplicationStream` actor which is used for maintaining a
//! replication stream from a Raft leader node to a target follower node.

mod heartbeat;
mod lagging;
mod linerate;
mod snapshot;

use std::sync::Arc;

use actix::prelude::*;

use crate::{
    AppData, AppDataResponse, AppError, NodeId,
    common::DependencyAddr,
    config::{Config, SnapshotPolicy},
    messages::{
        AppendEntriesRequest, AppendEntriesResponse,
        Entry, EntrySnapshotPointer, MembershipConfig,
    },
    network::RaftNetwork,
    raft::{Raft},
    storage::{RaftStorage, GetLogEntries},
};

//////////////////////////////////////////////////////////////////////////////////////////////////
// RSState ///////////////////////////////////////////////////////////////////////////////////////

/// The state of the replication stream.
enum RSState<D: AppData> {
    /// The replication stream is running at line rate.
    LineRate(LineRateState<D>),
    /// The replication stream is lagging behind due to the target node.
    Lagging(LaggingState<D>),
    /// The replication stream is streaming a snapshot over to the target node.
    Snapshotting(SnapshottingState),
}

/// LineRate specific state.
struct LineRateState<D: AppData> {
    /// A buffer of data to replicate to the target follower.
    ///
    /// The buffered payload here will be expanded as more replication commands come in from the
    /// Raft node while there is a buffered instance here.
    buffered_outbound: Vec<Arc<Entry<D>>>,
}

impl<D: AppData> Default for LineRateState<D> {
    fn default() -> Self {
        Self{buffered_outbound: vec![]}
    }
}

/// Lagging specific state.
struct LaggingState<D: AppData> {
    /// A flag indicating if the stream is ready to transition over to line rate.
    is_ready_for_line_rate: bool,
    /// A buffer of data to replicate to the target follower.
    ///
    /// This is identical to `LineRateState`'s buffer, and will be trasferred over to its buffer
    /// during state transition.
    buffered_outbound: Vec<Arc<Entry<D>>>,
}

impl<D: AppData> Default for LaggingState<D> {
    fn default() -> Self {
        Self{is_ready_for_line_rate: false, buffered_outbound: vec![]}
    }
}

/// Snapshotting specific state.
#[derive(Default)]
struct SnapshottingState;

//////////////////////////////////////////////////////////////////////////////////////////////////
// ReplicationStream /////////////////////////////////////////////////////////////////////////////

/// An actor responsible for sending replication events to a target follower in the Raft cluster.
///
/// This actor is spawned as part of a Raft node becoming the cluster leader. When the Raft node
/// is no longer the leader, it will drop the address of this actor which will cause this actor to
/// stop.
///
/// ### line rate replication
/// This actor is implemented in such a way that it will receive updates from the Raft node as
/// log entries are successfully appended to the log and flushed to disk. The Raft node will send
/// an `RSReplicate` payload of the log entries which were just appended to the log. With the
/// information in the payload, replication streams can stay at line rate a majority of the time.
///
/// When running at line rate, replication requests will be buffered when there is an outstanding
/// request, and all buffered entries will be sent over in the next request as one larger payload.
///
/// ### lagging replication
/// When a replication request fails (typically due to target being new to the cluster, the target
/// having been offline for some time, or any such reason), the replication stream will enter the
/// state `RSState::Lagging`, indicating that the target needs to be brought up-to-speed and that
/// it is no longer running at line rate. When such an event takes place, any buffered replication
/// payload will be purged, and the replication stream will begin the process of bringing the
/// target up-to-speed.
///
/// #### bringing target up-to-date
/// When the replication stream enters the `RSState::Lagging`, the replication stream will attempt
/// to bring the target up-to-date based on the type of failure which triggered the state change.
/// This Raft implementation uses a _conflict optimization_ algorithm as described in §5.3. As
/// such, a `ConflictOpt` struct should always be present to help determine the last index to
/// resume replication from.
///
/// If the node is far enough behind, based on the Raft's configuration, the target may need to be
/// sent an InstallSnapshot RPC. When this needs to take place, the replication stream will
/// transition to the state `RSState::Snapshotting`, and will then proceed to stream a
/// snapshot over to the target node.
///
/// #### back to line rate
/// When the replication stream has finished with the snapshot process and/or has fetched a
/// payload of entries which brings that node back up to line rate, before the payload is sent,
/// the replication stream will transition back to state `RSState::LineRate`. This allows the
/// replication stream to safely recover back to line rate even under heavy write load.
///
/// ----
///
/// NOTE: we do not stack replication requests to targets because this could result in
/// out-of-order delivery.
pub(crate) struct ReplicationStream<D: AppData, R: AppDataResponse, E: AppError, N: RaftNetwork<D>, S: RaftStorage<D, R, E>> {
    //////////////////////////////////////////////////////////////////////////
    // Static Fields /////////////////////////////////////////////////////////

    /// The ID of this Raft node.
    id: NodeId,
    /// The ID of the target Raft node which replication events are to be sent to.
    target: NodeId,
    /// The current term.
    ///
    /// This will never change while this actor is alive. If a newer term is observed, this actor
    /// will be brought down as part of a state transition to becoming a follower.
    term: u64,
    /// A channel for communicating with the Raft node which spawned this actor.
    raftnode: Addr<Raft<D, R, E, N, S>>,
    /// The address of the actor responsible for implementing the `RaftNetwork` interface.
    network: Addr<N>,
    /// The storage interface.
    storage: Recipient<GetLogEntries<D, E>>,
    /// The Raft's runtime config.
    config: Arc<Config>,

    //////////////////////////////////////////////////////////////////////////
    // Dynamic Fields ////////////////////////////////////////////////////////

    /// The state of this replication stream, primarily corresponding to replication performance.
    state: RSState<D>,
    /// A flag indicating if the state loop is currently being driven forward.
    is_driving_state: bool,
    /// The index of the log entry to most recently be appended to the log by the leader.
    line_index: u64,
    /// The index of the highest log entry which is known to be committed in the cluster.
    line_commit: u64,

    /// The index of the next log to send.
    ///
    /// This is initialized to leader's last log index + 1. Per the Raft protocol spec,
    /// this value may be decremented as new nodes enter the cluster and need to catch-up.
    ///
    /// If a follower’s log is inconsistent with the leader’s, the AppendEntries consistency check
    /// will fail in the next AppendEntries RPC. After a rejection, the leader decrements
    /// `next_index` and retries the AppendEntries RPC. Eventually `next_index` will reach a point
    /// where the leader and follower logs match. When this happens, AppendEntries will succeed,
    /// which removes any conflicting entries in the follower’s log and appends entries from the
    /// leader’s log (if any). Once AppendEntries succeeds, the follower’s log is consistent with
    /// the leader’s, and it will remain that way for the rest of the term.
    ///
    /// This Raft implementation also uses a _conflict optimization_ pattern for reducing the
    /// number of RPCs which need to be sent back and forth between a peer which is lagging
    /// behind. This is defined in §5.3.
    next_index: u64,
    /// The last know index to be successfully replicated on the target.
    ///
    /// This will be initialized to the leader's last_log_index, and will be updated as
    /// replication proceeds.
    match_index: u64,
    /// The term of the last know index to be successfully replicated on the target.
    ///
    /// This will be initialized to the leader's last_log_term, and will be updated as
    /// replication proceeds.
    match_term: u64,
}

impl<D: AppData, R: AppDataResponse, E: AppError, N: RaftNetwork<D>, S: RaftStorage<D, R, E>> ReplicationStream<D, R, E, N, S> {
    /// Create a new instance.
    pub fn new(
        id: NodeId, target: NodeId, term: u64, config: Arc<Config>,
        line_index: u64, line_term: u64, line_commit: u64,
        raftnode: Addr<Raft<D, R, E, N, S>>, network: Addr<N>, storage: Recipient<GetLogEntries<D, E>>,
    ) -> Self {
        Self{
            id, target, term, raftnode, network, storage, config,
            state: RSState::LineRate(Default::default()), is_driving_state: false,
            line_index, line_commit,
            next_index: line_index + 1, match_index: line_index, match_term: line_term,
        }
    }

    /// Drive the replication stream forward.
    ///
    /// This method will take into account the current state of the replication stream and will
    /// drive forward the next actions it needs to take based on the state.
    ///
    /// NOTE WELL: we can not attempt to make a determination about what state this stream
    /// "should" be in from here, as the individual drive_* methods will cause state changes based
    /// on error conditions and the like.
    fn drive_state(&mut self, ctx: &mut Context<Self>) {
        // TODO: once async/await lands and we have better async looping functionality,
        // turn this into an async function and call the other drive_* methods in a loop, instead
        // of having to juggle the `is_driving_state` bool so carefully.

        // If task already in progress, do nothing.
        if self.is_driving_state {
            return;
        }

        // Begin the next state pass.
        self.is_driving_state = true;
        match &self.state {
            RSState::LineRate(_) => self.drive_state_line_rate(ctx),
            RSState::Lagging(_) => self.drive_state_lagging(ctx),
            RSState::Snapshotting(_) => self.drive_state_snapshotting(ctx),
        }
    }

    /// Handle AppendEntries RPC responses from the target node.
    ///
    /// ### last_index_and_term
    /// An optional tuple of the index and term of the last entry to be appended per the
    /// corresponding request.
    fn handle_append_entries_response(
        &mut self, ctx: &mut Context<Self>, res: AppendEntriesResponse, last_index_and_term: Option<(u64, u64)>,
    ) -> Box<dyn ActorFuture<Actor=Self, Item=(), Error=()> + 'static> {
        // TODO: remove the allocations here once async/await lands on stable.

        // Handle success conditions.
        if res.success {
            // If this was a proper replication event (last index & term were provided), then update state.
            if let Some((index, term)) = last_index_and_term {
                self.next_index = index + 1; // This should always be the next expected index.
                self.match_index = index;
                self.match_term = term;
                self.raftnode.do_send(RSUpdateMatchIndex{target: self.target, match_index: index});
            }

            // If running at line rate, and our buffered outbound requests have accumulated too
            // much, we need to purge and transition to a lagging state. The target is not able to
            // replicate data fast enough.
            if let RSState::LineRate(inner) = &self.state {
                if inner.buffered_outbound.len() > (self.config.max_payload_entries as usize) {
                    return Box::new(self.transition_to_lagging(ctx));
                }
            }

            // Else, this was just a heartbeat. Do nothing.
            return Box::new(fut::ok(()));
        }

        // Replication was not successful, if a newer term has been returned, revert to follower.
        if &res.term > &self.term {
            return Box::new(
                fut::wrap_future(self.raftnode.send(RSRevertToFollower{target: self.target, term: res.term}))
                    .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftInternal))
                    // This condition represents a replication failure, so return an error condition.
                    .and_then(|_, _, ctx| {
                        ctx.terminate(); // Terminate this replication stream.
                        fut::err(())
                    }));
        }

        // Replication was not successful, handle conflict optimization record, else decrement `next_index`.
        if let Some(conflict) = res.conflict_opt {
            // If the returned conflict opt index is greater than line index, then this is a
            // logical error, and no action should be taken. This represents a replication failure.
            if &conflict.index > &self.line_index {
                return Box::new(fut::err(()));
            }

            // Check snapshot policy and handle conflict as needed.
            match &self.config.snapshot_policy {
                SnapshotPolicy::Disabled => {
                    self.next_index = conflict.index + 1;
                    self.match_index = conflict.index;
                    self.match_term = conflict.term;
                    return Box::new(self.transition_to_lagging(ctx));
                }
                SnapshotPolicy::LogsSinceLast(threshold) => {
                    let diff = &self.line_index - &conflict.index; // NOTE WELL: underflow is guarded against above.
                    let needs_snpshot = &diff >= threshold;
                    self.next_index = conflict.index + 1;
                    self.match_index = conflict.index;
                    self.match_term = conflict.term;

                    if needs_snpshot {
                        // Follower is far behind and needs to receive an InstallSnapshot RPC.
                        return Box::new(self.transition_to_snapshotting(ctx));
                    }
                    // Follower is behind, but not too far behind to receive an InstallSnapshot RPC.
                    return Box::new(self.transition_to_lagging(ctx));
                }
            }
        } else {
            self.next_index = if self.next_index > 0 { self.next_index - 1} else { 0 }; // Guard against underflow.
            return Box::new(self.transition_to_lagging(ctx));
        }
    }

    /// Transform and log an actix MailboxError.
    ///
    /// This method treats the error as being fatal, as Raft can not function properly if the
    /// `RaftNetowrk` & `RaftStorage` interfaces are returning mailbox errors. This method will
    /// shutdown the Raft actor.
    fn map_fatal_actix_messaging_error(&mut self, _: &mut Context<Self>, err: actix::MailboxError, dep: DependencyAddr) {
        self.raftnode.do_send(RSFatalActixMessagingError{target: self.target, err, dependency: dep})
    }

    /// Transform an log the result of a `RaftStorage` interaction.
    ///
    /// This method assumes that a storage error observed here is non-recoverable. As such, the
    /// Raft node will be instructed to stop. If such behavior is not needed, then don't use this
    /// interface.
    fn map_fatal_storage_result<T>(&mut self, _: &mut Context<Self>, res: Result<T, E>) -> impl ActorFuture<Actor=Self, Item=T, Error=()> {
        let res = res.map_err(|err| {
            self.raftnode.do_send(RSFatalStorageError{target: self.target, err});
        });
        fut::result(res)
    }

    /// Send the given AppendEntries RPC to the target & await the response.
    ///
    /// If a response successfully comes back from the target, the heartbeat timer will be
    /// updated. This routine does not perform any timeout logic. That is up to the parent
    /// application's networking layer.
    fn send_append_entries(
        &mut self, _: &mut Context<Self>, request: AppendEntriesRequest<D>,
    ) -> impl ActorFuture<Actor=Self, Item=AppendEntriesResponse, Error=()> {
        // Send the payload.
        fut::wrap_future(self.network.send(request))
            .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftNetwork))
            .and_then(|res, _, _| fut::result(res))
    }

    /// Transition this actor to the state `RSState::Lagging` & notify Raft node.
    ///
    /// NOTE WELL: this will not drive the state forward. That must be called from business logic.
    fn transition_to_lagging(&mut self, _: &mut Context<Self>) -> impl ActorFuture<Actor=Self, Item=(), Error=()> {
        self.state = RSState::Lagging(LaggingState::default());
        let event = RSRateUpdate{target: self.target, is_line_rate: false};
        fut::wrap_future(self.raftnode.send(event))
            .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftInternal))
    }

    /// Transition this actor to the state `RSState::LineRate` & notify Raft node.
    ///
    /// NOTE WELL: this will not drive the state forward. That must be called from business logic.
    fn transition_to_line_rate(&mut self, _: &mut Context<Self>) -> impl ActorFuture<Actor=Self, Item=(), Error=()> {
        // Transition pertinent state from lagging to line rate.
        let mut new_state = LineRateState::default();
        match &mut self.state {
            RSState::Lagging(inner) => {
                new_state.buffered_outbound.append(&mut inner.buffered_outbound);
            }
            _ => (),
        }
        self.state = RSState::LineRate(new_state);
        let event = RSRateUpdate{target: self.target, is_line_rate: true};
        fut::wrap_future(self.raftnode.send(event))
            .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftInternal))
    }

    /// Transition this actor to the state `RSState::Snapshotting` & notify Raft node.
    ///
    /// NOTE WELL: this will not drive the state forward. That must be called from business logic.
    fn transition_to_snapshotting(&mut self, _: &mut Context<Self>) -> impl ActorFuture<Actor=Self, Item=(), Error=()> {
        self.state = RSState::Snapshotting(SnapshottingState::default());
        let event = RSRateUpdate{target: self.target, is_line_rate: false};
        fut::wrap_future(self.raftnode.send(event))
            .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftInternal))
    }
}

impl<D: AppData, R: AppDataResponse, E: AppError, N: RaftNetwork<D>, S: RaftStorage<D, R, E>> Actor for ReplicationStream<D, R, E, N, S> {
    type Context = Context<Self>;

    /// Perform actors startup routine.
    ///
    /// As part of startup, the replication stream must send an empty AppendEntries RPC payload
    /// to its target to ensure that the target node is aware of the leadership state of this
    /// Raft node.
    fn started(&mut self, ctx: &mut Self::Context) {
        // Send initial heartbeat & perform first call to `drive_state`.
        let f = self.heartbeat_send(ctx).then(|res, act, ctx| {
            act.drive_state(ctx);
            fut::result(res)
        });
        ctx.spawn(f);
        self.setup_heartbeat(ctx);
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// RSReplicate ///////////////////////////////////////////////////////////////////////////////////

/// A replication stream message indicating a new payload of entries to be replicated.
#[derive(Clone)]
pub(crate) struct RSReplicate<D: AppData> {
    /// The new entry which needs to be replicated.
    ///
    /// This entry will always be the most recent entry to have been appended to the log, so its
    /// index is the new line index value.
    pub entry: Arc<Entry<D>>,
    /// The index of the highest log entry which is known to be committed in the cluster.
    pub line_commit: u64,
}

impl<D: AppData> Message for RSReplicate<D> {
    type Result = Result<(), ()>;
}

impl<D: AppData, R: AppDataResponse, E: AppError, N: RaftNetwork<D>, S: RaftStorage<D, R, E>> Handler<RSReplicate<D>> for ReplicationStream<D, R, E, N, S> {
    type Result = Result<(), ()>;

    /// Handle a request to replicate the given payload of entries.
    ///
    /// If there is already an outbound request, this payload will be buffered. If there is
    /// already a buffered payload, the payloads will be combined. This helps to keep throughput
    /// as high as possible when dealing with high write load conditions.
    fn handle(&mut self, msg: RSReplicate<D>, ctx: &mut Self::Context) -> Self::Result {
        // Always update line commit & index info first so that this value can be used in all AppendEntries RPCs.
        self.line_commit = msg.line_commit;
        self.line_index = msg.entry.index;

        // Get a mutable reference to an inner buffer if permitted by current state, else return.
        match &mut self.state {
            // NOTE: exceeding line rate buffer size is accounted for in the `handle_append_entries_response` handler.
            RSState::LineRate(inner) => inner.buffered_outbound.push(msg.entry),
            RSState::Lagging(inner) => inner.buffered_outbound.push(msg.entry),
            _ => return Ok(()),
        };

        self.drive_state(ctx);
        Ok(())
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// RSUpdateLineCommit ////////////////////////////////////////////////////////////////////////////

/// A replication stream message indicating a new payload of entries to be replicated.
#[derive(Clone, Message)]
pub(crate) struct RSUpdateLineCommit(pub u64);

impl<D: AppData, R: AppDataResponse, E: AppError, N: RaftNetwork<D>, S: RaftStorage<D, R, E>> Handler<RSUpdateLineCommit> for ReplicationStream<D, R, E, N, S> {
    type Result = ();

    /// Handle a request to update the current line commit of the leader.
    fn handle(&mut self, msg: RSUpdateLineCommit, _: &mut Self::Context) -> Self::Result {
        self.line_commit = msg.0;
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// RSTerminate ///////////////////////////////////////////////////////////////////////////////////

/// A replication stream message indicating a new payload of entries to be replicated.
#[derive(Message)]
pub(crate) struct RSTerminate;

impl<D: AppData, R: AppDataResponse, E: AppError, N: RaftNetwork<D>, S: RaftStorage<D, R, E>> Handler<RSTerminate> for ReplicationStream<D, R, E, N, S> {
    type Result = ();

    /// Handle a request to terminate this replication stream.
    fn handle(&mut self, _: RSTerminate, ctx: &mut Self::Context) -> Self::Result {
        ctx.terminate();
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// RSFatalStorageError ///////////////////////////////////////////////////////////////////////////

/// An event representing a fatal storage error.
#[derive(Message)]
pub(crate) struct RSFatalStorageError<E: AppError> {
    /// The ID of the Raft node which this event relates to.
    pub target: NodeId,
    /// The storage error which produced this event.
    pub err: E,
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// RSFatalActixMessagingError ////////////////////////////////////////////////////////////////////

/// An event representing a fatal actix messaging error.
#[derive(Message)]
pub(crate) struct RSFatalActixMessagingError {
    /// The ID of the Raft node which this event relates to.
    pub target: NodeId,
    /// The actix mailbox error which produced this event.
    pub err: MailboxError,
    /// The dependency responsible for producing the error.
    pub dependency: DependencyAddr,
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// RSRateUpdate //////////////////////////////////////////////////////////////////////////////////

/// An event representing an update to the replication rate of a replication stream.
#[derive(Message)]
pub(crate) struct RSRateUpdate {
    /// The ID of the Raft node which this event relates to.
    pub target: NodeId,
    /// A flag indicating if the corresponding target node is replicating at line rate.
    ///
    /// When replicating at line rate, the replication stream will receive log entires to
    /// replicate as soon as they are ready. When not running at line rate, the Raft node will
    /// only send over metadata without entries to replicate.
    pub is_line_rate: bool,
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// RSRevertToFollower ////////////////////////////////////////////////////////////////////////////

/// An event indicating that the Raft node needs to rever to follower state.
#[derive(Message)]
pub(crate) struct RSRevertToFollower {
    /// The ID of the target node from which the new term was observed.
    pub target: NodeId,
    /// The new term observed.
    pub term: u64,
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// RSNeedsSnapshot ///////////////////////////////////////////////////////////////////////////////

/// An event from a replication stream requesting snapshot info.
pub(crate) struct RSNeedsSnapshot;

impl Message for RSNeedsSnapshot {
    type Result = Result<RSNeedsSnapshotResponse, ()>;
}

/// A resonse from the Raft actor with information on the current snapshot.
pub(crate) struct RSNeedsSnapshotResponse {
    pub index: u64,
    pub term: u64,
    pub membership: MembershipConfig,
    pub pointer: EntrySnapshotPointer,
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// RSUpdateMatchIndex ////////////////////////////////////////////////////////////////////////////

/// An event from a replication stream which updates the target node's match index.
#[derive(Message)]
pub(crate) struct RSUpdateMatchIndex {
    /// The ID of the target node for which the match index is to be updated.
    pub target: NodeId,
    /// The index of the most recent log known to have been successfully replicated on the target.
    pub match_index: u64,
}
