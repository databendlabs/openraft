//! Replication stream actors.
//!
//! This module encapsulates the `ReplicationStream` actor which is used for maintaining a
//! replication stream from a Raft leader node to a target follower node.


use std::sync::Arc;

use actix::prelude::*;
use log::{error, info, warn};
use tokio::{
    prelude::*,
    fs::File,
};

use crate::{
    config::{Config, SnapshotPolicy},
    proto,
    raft::{NodeId, Raft, RaftRpcOut},
    storage::RaftStorage,
};

const MAILBOX_ERR_MESSAGE: &str = "Error communicating from ReplicationStream to Raft node.";

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
/// `RSState::Lagging` state, indicating that the target needs to be brought up-to-speed and that
/// it is no longer running at line rate. When such an event takes place, any buffered replication
/// payload will be purged, and the replication stream will begin the process of bringing the
/// target up-to-speed. The replication stream will notify the Raft node that it is no longer
/// running at line rate, and the Raft node will stop sending entries in the `RSReplicate`
/// messages until the target is caught up and ready to resume running at line rate.
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
/// transition to the state `RSState::InstallingSnapshot`, and will then proceed to stream a
/// snapshot over to the target node.
///
/// #### back to line rate
/// When the replication stream has finished with the snapshot process and/or has fetched a
/// payload of entries to send to the target — before the payload is sent — the replication stream
/// will transition back to state `RSState::LineRate`. This allows the replication stream to
/// safely recover back to line rate even under heavy write load.
///
/// ----
///
/// NOTE: we do not stack replication requests to targets because this could result in
/// out-of-order delivery.
pub(crate) struct ReplicationStream<S: RaftStorage> {
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
    raftnode: Addr<Raft<S>>,
    /// The output channel used for sending RPCs to the transport/network layer.
    out: Recipient<RaftRpcOut>,
    /// The Raft's runtime config.
    config: Arc<Config>,

    //////////////////////////////////////////////////////////////////////////
    // Dynamic Fields ////////////////////////////////////////////////////////

    /// The state of this replication stream, primarily corresponding to replication performance.
    state: RSState,
    /// The index of the log entry to most recently be appended to the log by the leader.
    line_index: u64,
    /// The index of the highest log entry which is known to be committed in the cluster.
    line_commit: u64,
    /// The index of the next log to send.
    ///
    /// Each entry is initialized to leader's last log index + 1. Per the Raft protocol spec,
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
    /// An optional handle to the current outbound request to the target node.
    current_request: Option<SpawnHandle>,
    /// An optional handle to the current heartbeat timeout operation.
    ///
    /// This does not represent the heartbeat request itself, it represents the timer which will
    /// trigger a heartbeat in the future.
    heartbeat: Option<SpawnHandle>,
    /// An optional buffered payload of data to replicate to the target follower.
    ///
    /// The buffered payload here will be expanded as more replication commands come in from the
    /// Raft node while there is a buffered instance here.
    buffered_outbound_payload: Option<RSReplicate>,
}

impl<S: RaftStorage> ReplicationStream<S> {
    /// Drive the replication process forward.
    ///
    /// This method is usually called when a replication request is received or when a request to
    /// the target node is finished. This will perform a check to see if there is a buffered
    /// payload which needs to be sent to the target. If there is a buffered payload and no
    /// outstanding request, then the payload will be sent immediately; else, replication will be
    /// driven forward after the current request resolves.
    fn drive_replication(&mut self, ctx: &mut Context<Self>) {
        // If there is an outstanding request, or we are not runing at line rate, do nothing.
        if self.current_request.is_some() || &self.state != &RSState::LineRate {
            return;
        }

        // There is no outstanding request; if there is a buffered payload, send it, else return.
        let buffered = match self.buffered_outbound_payload.take() {
            None => return, // Nothing to do, so return.
            Some(payload) => payload,
        };
        let payload = proto::AppendEntriesRequest{
            term: self.term,
            leader_id: self.id,
            prev_log_index: self.line_index,
            prev_log_term: self.term,
            entries: buffered.entries,
            leader_commit: self.line_commit,
        };
        self.send_payload(ctx, payload);
    }

    /// Handle AppendEntries RPC responses from the target node.
    fn handle_append_entries_response(&mut self, ctx: &mut Context<Self>, res: proto::RaftResponse, payload_last_index: Option<u64>) {
        // Unpack the inner payload.
        let payload = match res.payload {
            Some(proto::raft_response::Payload::AppendEntries(inner)) => inner,
            _ => {
                warn!("Unexpected RPC response from {} after send an AppendEntries request. {:?}", self.target, &res.payload);
                return;
            }
        };

        // If the response indicates a success, then we are done here.
        if payload.success {
            // If this was a proper replication event, notify Raft node.
            if let Some(idx) = payload_last_index {
                self.next_index = idx + 1; // This should always be the next expected index.
                let event = RSEvent::Replication(RSEventReplication{
                    target: self.target,
                    state: self.state.clone(),
                    last_log_index: Some(idx),
                });
                ctx.spawn(fut::wrap_future(self.raftnode.send(event))
                    .map_err(|err, act: &mut Self, _| error!("{} {} {}", MAILBOX_ERR_MESSAGE, act.target, err)));
            }

            // Else, this was just a heartbeat. Do nothing.
            return;
        }

        // Replication was not successful, if a newer term has been returned, revert to follower.
        if &payload.term > &self.term {
            ctx.spawn(fut::wrap_future(self.raftnode.send(RSEvent::RevertToFollower(self.target, payload.term)))
                .map_err(|err, act: &mut Self, _| error!("{} {} {}", MAILBOX_ERR_MESSAGE, act.target, err)));
        }

        // Replication was not successful, handle conflict optimization record, else decrement `next_index`.
        if let Some(conflict) = payload.conflict_opt {
            // If the returned conflict opt index is greater than line index, then this is a
            // logical error, and no action should be taken.
            if &conflict.index > &self.line_index {
                return;
            }

            // Check snapshot policy and handle conflict as needed.
            match &self.config.snapshot_policy {
                SnapshotPolicy::Disabled => {
                    self.next_index = conflict.index;
                    self.transition_to_lagging(ctx);
                }
                SnapshotPolicy::LogsSinceLast(threshold) => {
                    let diff = &self.line_index - &conflict.index;
                    if &diff >= threshold {
                        // Follower is far behind and needs to receive an InstallSnapshot RPC.
                        self.transition_to_installing_snapshot(ctx);
                    } else {
                        // Follower is behind, but not too far behind to receive an InstallSnapshot RPC.
                        self.next_index = conflict.index;
                        self.transition_to_lagging(ctx);
                    }
                }
            }
        } else {
            self.next_index -= 1;
            self.transition_to_lagging(ctx);
        }
    }

    /// Handle a request to replicate the given payload of entries.
    ///
    /// If there is already an outbound request, this payload will be buffered. If there is
    /// already a buffered payload, the payloads will be combined. This helps to keep throughput
    /// as high as possible when dealing with high write load conditions.
    fn handle_rsreplicate_message(&mut self, ctx: &mut Context<Self>, mut msg: RSReplicate) {
        // Always update line commit & index info first so that this value can be used in all AppendEntries RPCs.
        self.line_commit = msg.line_commit;
        self.line_index = msg.line_index;

        // If there is a buffered replication payload, update it and finish this up.
        if let Some(payload) = &mut self.buffered_outbound_payload {
            payload.line_commit = msg.line_commit;
            payload.line_index = msg.line_index;
            payload.entries.append(&mut msg.entries);
        } else {
            self.buffered_outbound_payload = Some(msg);
        }

        self.drive_replication(ctx);
    }

    /// Send a heartbeat frame to the target node.
    fn send_heartbeat(&mut self, ctx: &mut Context<Self>) {
        // Don't do anything if there is already a request sent to the target.
        if self.current_request.is_some() {
            return;
        }

        // Prioritize sending entries if we have a buffered payload.
        if self.buffered_outbound_payload.is_some() {
            self.drive_replication(ctx);
            return;
        }

        // Build the heartbeat frame to be sent to the follower.
        let payload = proto::AppendEntriesRequest{
            term: self.term,
            leader_id: self.id,
            prev_log_index: self.line_index,
            prev_log_term: self.term,
            entries: Vec::with_capacity(0),
            leader_commit: self.line_commit,
        };
        self.send_payload(ctx, payload);
    }

    /// Unconditionally send the given payload to the target.
    ///
    /// This will spawn a new future for the operation of sending the payload to the target peer.
    /// The spawn handle from that future will be bound to `current_request`. The request will
    /// timeout after `heartbeat_rate`.
    ///
    /// After the request resolves, the spawn handle in `current_request` will be dropped (not
    /// cancelled), `update_heartbeat` will be called, and `drive_replication` will be called to
    /// ensure that any buffered payload ready to be sent will immediately be sent.
    fn send_payload(&mut self, ctx: &mut Context<Self>, request: proto::AppendEntriesRequest) {
        let payload_last_index = request.entries.last().map(|val| val.index);
        let payload = RaftRpcOut{
            target: self.target,
            request: proto::RaftRequest::new_append(request),
        };

        // Send the payload.
        let timeout = std::time::Duration::from_millis(self.config.heartbeat_interval);
        let handle = ctx.spawn(fut::wrap_future(self.out.send(payload))
            // Setup timeout.
            .timeout(timeout, MailboxError::Timeout)
            // Handle initial error conditions.
            .map_err(|err, act: &mut Self, _| match err {
                MailboxError::Closed => error!("Error while sending replication frames to {}. {:?}", act.target, err),
                MailboxError::Timeout => warn!("Replication request to node {} timedout. Timeout config {:?}ms. {:?}", act.id, act.config.heartbeat_interval, err),
            })
            // Flatten inner result.
            .and_then(|res, _, _| fut::FutureResult::from(res))
            // Process the response.
            .map(move |res, act, ctx| act.handle_append_entries_response(ctx, res, payload_last_index))
            // Remove the retained spawn handle, update heartbeat & drive the replication process forward.
            .then(|res, act, ctx| {
                act.current_request.take();
                act.update_heartbeat(ctx);
                act.drive_replication(ctx);
                fut::FutureResult::from(res)
            }));
        self.current_request = Some(handle);
    }

    /// Send the specified snapshot over to the target node.
    fn send_snapshot(&mut self, _: &mut Context<Self>, snap: InstallSnapshotResponse) -> impl ActorFuture<Actor=Self, Item=(), Error=()> {
        // Ensure state is still `RSState::InstallingSnapshot`.
        if &self.state != &RSState::InstallingSnapshot {
            return fut::Either::A(fut::FutureResult::from(Ok(())));
        }

        // Look up the snapshot on disk.
        fut::Either::B(fut::wrap_future(tokio::fs::File::open(snap.pointer.path.clone()))
            .map_err(|err, _, _| error!("Error opening snapshot file. {}", err))
            // We've got a successful file handle, start streaming the chunks over to the target.
            .and_then(move |file, act: &mut Self, _| {
                // Create a snapshot stream from the file and stream it over to the target.
                let snap_stream = SnapshotStream::new(file, act.config.snapshot_max_chunk_size as usize, act.term, act.id, snap.index, snap.term);
                fut::wrap_stream(snap_stream)
                    .map_err(|err, act: &mut Self, _| error!("Error while streaming snapshot to target {}: {}", &act.target, err))
                    .and_then(|rpc, act: &mut Self, _| {
                        // Send snapshot RPC frame over to target.
                        fut::wrap_future(act.out.send(RaftRpcOut{
                            target: act.target,
                            request: proto::RaftRequest{payload: Some(proto::raft_request::Payload::InstallSnapshot(rpc))},
                        }))
                        .map_err(|err, _: &mut Self, _| error!("Error while sending outbound InstallSnapshot RPC: {}", err))
                        .and_then(|res, _, _| fut::FutureResult::from(res))
                        // Handle response from target.
                        .and_then(|res, act, ctx| act.handle_install_snapshot_response(ctx, res))
                    })
                    .finish()
            })
            // If we've run into an error during the above process, we restart the process.
            .map_err(|_, act, ctx| act.transition_to_installing_snapshot(ctx)))
    }

    /// Handle frames from the target node which are in response to an InstallSnapshot RPC request.
    fn handle_install_snapshot_response(&mut self, _: &mut Context<Self>, res: proto::RaftResponse) -> impl ActorFuture<Actor=Self, Item=(), Error=()> {
        // Unpack inner payload.
        use proto::raft_response::Payload;
        match res.payload {
            // Check the response term. As long as everything still matches, then we are good to resume.
            Some(Payload::InstallSnapshot(inner)) => if &inner.term == &self.term {
                fut::FutureResult::from(Ok(()))
            } else {
                info!("Response from InstallSnapshot RPC sent to {} indicates a newer term {} is in session, reverting to follower.", &self.target, &inner.term);
                self.raftnode.do_send(RSEvent::RevertToFollower(self.target, inner.term));
                fut::FutureResult::from(Err(()))
            },
            _ => {
                warn!("Unexpected RPC response type from {} after sending InstallSnapshot RPC.", self.target);
                fut::FutureResult::from(Err(()))
            }
        }
    }

    /// Transition this actor to the state `RSState::Lagging` & notify Raft node.
    fn transition_to_lagging(&mut self, ctx: &mut Context<Self>) {
        self.state = RSState::Lagging;
        self.buffered_outbound_payload.take(); // Remove any buffered payload.
        let event = RSEventReplication{target: self.target, state: self.state.clone(), last_log_index: None};
        ctx.spawn(fut::wrap_future(self.raftnode.send(RSEvent::Replication(event)))
            .map_err(|err, act: &mut Self, _| error!("{} {} {}", MAILBOX_ERR_MESSAGE, act.target, err)));

        // TODO: RESUME HERE IMMEDIATE <<<<<<<<
    }

    /// Transition this actor to the state `RSState::InstallingSnapshot` & notify Raft node.
    fn transition_to_installing_snapshot(&mut self, ctx: &mut Context<Self>) {
        self.state = RSState::InstallingSnapshot;
        self.buffered_outbound_payload.take(); // Remove any buffered payload.
        let event = InstallSnapshot{target: self.target};
        let f = fut::wrap_future(self.raftnode.send(event))
            // Log actix messaging errors.
            .map_err(|err, act: &mut Self, _| error!("{} {} {}", MAILBOX_ERR_MESSAGE, act.target, err))
            // Flatten inner result.
            .and_then(|res, _, _| fut::FutureResult::from(res))
            // Handle response from Raft node and start streaming over the snapshot.
            .and_then(|res, act, ctx| act.send_snapshot(ctx, res))
            // If an error has come up, and we are still in snapshot state, simply retry the operation.
            .map_err(|_, act, ctx| {
                if &act.state == &RSState::InstallingSnapshot {
                    act.transition_to_installing_snapshot(ctx);
                }
            });
        ctx.spawn(f);
    }

    /// Update the heartbeat mechanism.
    ///
    /// When called, this method will cancel any pending heartbeat and will schedule a new one
    /// based on the configured `heartbeat_rate`. Once a request is successfully returned from the
    /// target, this method should be called to ensure a timeout isn't hit, and also to ensure
    /// this system is not sending more network requests than is needed.
    fn update_heartbeat(&mut self, ctx: &mut Context<Self>) {
        // Remove pending heartbeat if present.
        if let Some(hb) = self.heartbeat.take() {
            ctx.cancel_future(hb);
        }

        // Setup a new heartbeat to be sent after the `heartbeat_rate` duration.
        let later = std::time::Duration::from_millis(self.config.heartbeat_interval);
        let handle = ctx.run_later(later, |act, ctx| act.send_heartbeat(ctx));
        self.heartbeat = Some(handle);
    }
}

impl<S: RaftStorage> Actor for ReplicationStream<S> {
    type Context = Context<Self>;

    /// Perform actors startup routine.
    ///
    /// As part of startup, the replication stream must send an empty AppendEntries RPC payload
    /// to its target to ensure that the target node is aware of the leadership state of this
    /// Raft node.
    fn started(&mut self, ctx: &mut Self::Context) {
        // Send initial heartbeat.
        self.send_heartbeat(ctx);
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// RSReplicate ///////////////////////////////////////////////////////////////////////////////////

/// A replication stream message indicating a new payload of entries to be replicated.
#[derive(Clone)]
pub(crate) struct RSReplicate {
    /// The payload of entries to be replicated.
    ///
    /// The payload will only be empty if the Raft node detects that this replication stream is
    /// not running at line rate, as buffering too many entries could cause memory issues.
    entries: Vec<proto::Entry>,
    /// The index of the log entry to most recently be appended to the log by the leader.
    ///
    /// This will always be the index of the last element in the entries payload if values are
    /// given in the payload.
    line_index: u64,
    /// The index of the highest log entry which is known to be committed in the cluster.
    line_commit: u64,
}

impl Message for RSReplicate {
    type Result = Result<(), ()>;
}

impl<S: RaftStorage> Handler<RSReplicate> for ReplicationStream<S> {
    type Result = Result<(), ()>;

    /// Handle replication requests from the Raft node.
    fn handle(&mut self, msg: RSReplicate, ctx: &mut Self::Context) -> Self::Result {
        self.handle_rsreplicate_message(ctx, msg);
        Ok(())
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// RSEvent ///////////////////////////////////////////////////////////////////////////////////////

/// A replication stream message indicating an event coming from a replication stream.
#[derive(Message)]
pub(crate) enum RSEvent {
    /// An event representing some replication stream update.
    Replication(RSEventReplication),
    /// A newer term was observed in a response from a replication request, become a follower.
    ///
    /// This variant simply wraps the ID of the node from which the newer term was observed along
    /// with the value of the new term.
    RevertToFollower(NodeId, u64),
}

/// An event representing some replication stream update.
pub(crate) struct RSEventReplication {
    /// The ID of the Raft node which this RSEvent relates to.
    target: NodeId,
    /// The state of the replication stream.
    state: RSState,
    /// The index of the last log which was successfully replicated as part of this event.
    ///
    /// This value may be `None` due to some state changes related to conflicts or snapshots. If
    /// this value is `None`, then the information here is indicative of the replication stream
    /// transitioning to the specified state.
    last_log_index: Option<u64>,
}

/// The rate of a replication stream.
#[derive(Clone, Eq, PartialEq)]
pub(crate) enum RSState {
    /// The replication stream is running at line rate.
    LineRate,
    /// The replication stream is lagging behind due to the target node.
    Lagging,
    /// The replication stream is streaming a snapshot over to the target node.
    InstallingSnapshot,
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// InstallSnapshot ///////////////////////////////////////////////////////////////////////////////

/// An event from a replication stream requesting a snapshot to be installed on the target.
pub(crate) struct InstallSnapshot {
    /// The ID of the Raft node which this RSEvent relates to.
    target: NodeId,
}

impl Message for InstallSnapshot {
    type Result = Result<InstallSnapshotResponse, ()>;
}

/// A resonse from the Raft node for requesting to install a snapshot on a target.
pub(crate) struct InstallSnapshotResponse {
    pub index: u64,
    pub term: u64,
    pub pointer: proto::EntrySnapshotPointer,
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// SnapshotStream ////////////////////////////////////////////////////////////////////////////////

/// An async stream of the chunks of a given file.
struct SnapshotStream {
    file: File,
    offset: u64,
    buffer: Vec<u8>,
    term: u64,
    leader_id: NodeId,
    last_included_index: u64,
    last_included_term: u64,
    has_sent_terminal_frame: bool,
}

impl SnapshotStream {
    /// Create a new instance.
    pub fn new(file: File, bufsize: usize, term: u64, leader_id: NodeId, last_included_index: u64, last_included_term: u64) -> Self {
        Self{
            file, offset: 0, buffer: Vec::with_capacity(bufsize),
            term, leader_id, last_included_index, last_included_term,
            has_sent_terminal_frame: false,
        }
    }
}

impl futures::Stream for SnapshotStream {
    type Item = proto::InstallSnapshotRequest;
    type Error = tokio::io::Error;

    fn poll(&mut self) -> futures::Poll<Option<Self::Item>, Self::Error> {
        self.file.read_buf(&mut self.buffer).map(move |pollres| match pollres {
            Async::NotReady => Async::NotReady,
            Async::Ready(bytes_read) => {
                // The stream is ready to yield the next item.
                if bytes_read > 0 {
                    self.offset += bytes_read as u64;
                    let data = self.buffer.split_off(0); // Allocates a new buf only when we have successfully read bytes.
                    let frame = proto::InstallSnapshotRequest{
                        term: self.term, leader_id: self.leader_id,
                        last_included_index: self.last_included_index,
                        last_included_term: self.last_included_term,
                        offset: self.offset, data, done: false,
                    };
                    Async::Ready(Some(frame))
                }

                // There are no bytes left in the file, but we need to send the terminal RPC frame,
                // so mark the stream as being complete and send the terminal frame.
                else if !self.has_sent_terminal_frame {
                    self.has_sent_terminal_frame = true;
                    let frame = proto::InstallSnapshotRequest{
                        term: self.term, leader_id: self.leader_id,
                        last_included_index: self.last_included_index,
                        last_included_term: self.last_included_term,
                        offset: self.offset, data: Vec::with_capacity(0), done: true,
                    };
                    Async::Ready(Some(frame))
                }

                // There are no bytes left in the file, and we've already sent the terminal frame. We're done.
                else {
                    Async::Ready(None)
                }
            }
        })
    }
}
