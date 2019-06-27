//! Replication stream actors.
//!
//! This module encapsulates the `ReplicationStream` actor which is used for maintaining a
//! replication stream from a Raft leader node to a target follower node.

use actix::prelude::*;
use log::{error, warn};

use crate::{
    proto,
    raft::{
        NodeId,
        RaftRpcOut,
    },
};

/// An actor responsible for sending replication events to a target follower in the Raft cluster.
///
/// This actor is spawned as part of a Raft node becoming the cluster leader. When the Raft node
/// is no longer the leader, it will drop the address of this actor which will cause this actor to
/// stop.
///
/// This actor is implemented in such a way that it will receive updates from the parent node as
/// log entries are successfully appended to the log and flushed to disk. The Raft node will send
/// the payload of log entries which were just appended along with the index immediately of the
/// first entry, the index of the last entry, and the current commit index of the leader. With
/// this information, replication streams can stay at line rate a majority of the time, and when
/// follower is lagging behind, or is brand new to the cluster, the replication stream can bring
/// the node up-to-speed, or even start the snapshot process for the follower.
pub(crate) struct ReplicationStream {
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
    raftnode: Recipient<RSEvent>,
    /// The output channel used for sending RPCs to the transport/network layer.
    out: Recipient<RaftRpcOut>,
    /// The rate at which heartbeats will be sent to followers to avoid election timeouts.
    ///
    /// This value is also used as a timeout. This ensures that requests don't get backed up due
    /// to a logic error in the parent application's networking code.
    heartbeat_rate: u64,

    //////////////////////////////////////////////////////////////////////////
    // Dynamic Fields ////////////////////////////////////////////////////////

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
}

impl ReplicationStream {
    /// Send a heartbeat frame to the target node.
    fn send_heartbeat(&mut self, ctx: Context<Self>) {
        // Build the heartbeat frame to be sent to the follower.
        let frame = RaftRpcOut{
            target: self.target,
            request: proto::RaftRequest::new_append(proto::AppendEntriesRequest{
                term: self.term,
                leader_id: self.id,
                prev_log_index: self.line_index,
                prev_log_term: self.term,
                entries: Vec::with_capacity(0),
                leader_commit: self.line_commit,
            }),
        };

        let timeout = std::time::Duration::from_micros(self.heartbeat_rate);
        let f = fut::wrap_future(self.out.send(frame))
            .map_err(|err, act: &mut Self, _| error!("Actix messaging error while sending heartbeat to {}: {}", act.target, err))
            .timeout(timeout, ())
            .and_then(|res, _, _| fut::FutureResult::from(res))
            .map(|res, act, ctx| act.handle_append_entries_response(ctx, res, None))
            ;
    }

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
                let event = RSEvent::Replicated(RSEventReplicated{
                    target: self.target,
                    stream_rate: if idx == self.line_index { RSRate::Line } else { RSRate::Lagging },
                    last_log_index: idx,
                });
                ctx.spawn(fut::wrap_future(self.raftnode.send(event))
                    .map_err(|err, act: &mut Self, _| error!("Error communicating from {} ReplicationStream to Raft node. {}", act.target, err)));
            }

            // Else, this was just a heartbeat. Nothing to do.
            return;
        }

        // Replication was not successful, if a newer term has been returned, revert to follower.
        if payload.term > self.term {
            ctx.spawn(fut::wrap_future(self.raftnode.send(RSEvent::RevertToFollower))
                .map_err(|err, act: &mut Self, _| error!("Error communicating from {} ReplicationStream to Raft node. {}", act.target, err)));
        }

        // TODO: handle conflict opt. Even if no conflict opt has been presented, we still need to decrement `next_index`.
    }
}

impl Actor for ReplicationStream {
    type Context = Context<Self>;

    /// Perform actors startup routine.
    ///
    /// As part of startup, the replication stream must send an empty AppendEntries RPC payload
    /// to its target to ensure that the target node is aware of the leadership state of this
    /// Raft node.
    fn started(&mut self, _: &mut Self::Context) {
        // Send initial heartbeat.

        // Start individual heartbeat for this target.
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// RSReplicate ///////////////////////////////////////////////////////////////////////////////////

/// A replication stream message indicating a new payload of entries to be replicated.
#[derive(Clone, Message)]
pub(crate) struct RSReplicate {
    /// The payload of entries to be replicated.
    entries: Vec<proto::Entry>,
    /// The index of the log entry to most recently be appended to the log by the leader.
    line_index: u64,
    /// The index of the highest log entry which is known to be committed in the cluster.
    line_commit: u64,
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// RSEvent ///////////////////////////////////////////////////////////////////////////////////////

/// A replication stream message indicating an event coming from a replication stream.
#[derive(Message)]
pub(crate) enum RSEvent {
    /// An event indicating a successful replication event on a target node.
    Replicated(RSEventReplicated),
    /// A newer term was observed in a response from a replication request, become a follower.
    RevertToFollower,
}

pub(crate) struct RSEventReplicated {
    /// The ID of the Raft node which this RSEvent relates to.
    target: NodeId,
    /// The rate of the replication stream.
    stream_rate: RSRate,
    /// The index of the last log which was successfully replicated as part of this event.
    last_log_index: u64,
}

/// The rate of a replication stream.
pub(crate) enum RSRate {
    /// The replication stream is running at line rate.
    Line,
    /// The replication stream is lagging behind due to follower state.
    Lagging,
}
