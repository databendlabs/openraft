use actix::prelude::*;
use log::{warn};
use futures::sync::mpsc;

use crate::{
    AppError,
    common::{DependencyAddr, UpdateCurrentLeader},
    network::RaftNetwork,
    messages::{InstallSnapshotRequest, InstallSnapshotResponse},
    raft::{RaftState, Raft, SnapshotState},
    storage::{InstallSnapshot, InstallSnapshotChunk, RaftStorage},
};

impl<E: AppError, N: RaftNetwork<E>, S: RaftStorage<E>> Handler<InstallSnapshotRequest> for Raft<E, N, S> {
    type Result = ResponseActFuture<Self, InstallSnapshotResponse, ()>;

    /// Invoked by leader to send chunks of a snapshot to a follower (§7).
    ///
    /// Leaders always send chunks in order. It is important to note that, according to the Raft spec,
    /// a log may only have one snapshot at any time. As snapshot contents are application specific,
    /// the Raft log will only store a pointer to the snapshot file along with the index & term.
    ///
    /// See the `storage::InstallSnapshot` type for implementaion details.
    fn handle(&mut self, msg: InstallSnapshotRequest, ctx: &mut Self::Context) -> Self::Result {
        // Only handle requests if actor has finished initialization.
        if let &RaftState::Initializing = &self.state {
            warn!("Received Raft RPC before initialization was complete.");
            return Box::new(fut::err(()));
        }

        // Don't interact with non-cluster members.
        if !self.members.contains(&msg.leader_id) {
            return Box::new(fut::err(()));
        }

        // If message's term is less than most recent term, then we do not honor the request.
        if &msg.term < &self.current_term {
            return Box::new(fut::err(()));
        }

        // Update election timeout & ensure we are in the follower state. Update current term if needed.
        self.update_election_timeout(ctx);
        if &msg.term > &self.current_term || self.current_leader.as_ref() != Some(&msg.leader_id) {
            self.current_term = msg.term;
            self.become_follower(ctx);
            self.update_current_leader(ctx, UpdateCurrentLeader::OtherNode(msg.leader_id));
            self.save_hard_state(ctx);
        }

        // Extract follower specific state.
        let state = match &mut self.state {
            RaftState::Follower(state) => state,
            _ => return Box::new(fut::err(())),
        };

        // Compare current snapshot state with received RPC and handle as needed.
        match &mut state.snapshot_state {
            // Start streaming in a new snapshot.
            SnapshotState::Idle if &msg.offset == &0 && !msg.done => {
                let (tx, rx) = mpsc::unbounded();
                state.snapshot_state = SnapshotState::Streaming(tx);
                Box::new(fut::wrap_future(self.storage.send(InstallSnapshot::new(msg.last_included_term, msg.last_included_index, rx)))
                    .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftStorage))
                    .and_then(|res, act, ctx| act.map_fatal_storage_result(ctx, res))
                    .map(|_, act, _| InstallSnapshotResponse{term: act.current_term}))
            },
            SnapshotState::Idle => {
                warn!("Received an InstallSnapshot RPC which was not at a logical starting point.");
                Box::new(fut::err(()))
            }
            // If we're done streaming in the new snapshot, send in the final message and
            // transition to Idle state.
            SnapshotState::Streaming(tx) if msg.done => {
                let term = self.current_term;
                Box::new(fut::result(tx.unbounded_send(InstallSnapshotChunk{offset: msg.offset, data: msg.data, done: msg.done}))
                    .map_err(|_, _: &mut Self, _| ())
                    .and_then(move |_, act, _| {
                        match &mut act.state {
                            RaftState::Follower(state) => {
                                state.snapshot_state = SnapshotState::Idle;
                                fut::ok(InstallSnapshotResponse{term})
                            }
                            _ => fut::err(()),
                        }
                    }))
            }
            // Pipe a new snapshot chunk through the stream.
            SnapshotState::Streaming(tx) => {
                let term = self.current_term;
                tx.unbounded_send(InstallSnapshotChunk{offset: msg.offset, data: msg.data, done: msg.done})
                    .map(move |_| Box::new(fut::ok(InstallSnapshotResponse{term})))
                    .unwrap_or(Box::new(fut::err(())))
            }
        }
    }
}
