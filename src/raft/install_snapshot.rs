use actix::prelude::*;
use log::{debug, error};
use futures::sync::{mpsc, oneshot};

use crate::{
    AppData, AppError,
    common::{DependencyAddr, UpdateCurrentLeader},
    network::RaftNetwork,
    messages::{InstallSnapshotRequest, InstallSnapshotResponse},
    raft::{RaftState, Raft, SnapshotState},
    storage::{InstallSnapshot, InstallSnapshotChunk, RaftStorage},
};

impl<D: AppData, E: AppError, N: RaftNetwork<D>, S: RaftStorage<D, E>> Handler<InstallSnapshotRequest> for Raft<D, E, N, S> {
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
            return Box::new(fut::err(()));
        }

        // Don't interact with non-cluster members.
        if !self.membership.contains(&msg.leader_id) {
            return Box::new(fut::err(()));
        }

        // If message's term is less than most recent term, then we do not honor the request.
        if &msg.term < &self.current_term {
            return Box::new(fut::err(()));
        }

        // Update election timeout.
        self.update_election_timeout(ctx);

        // Update current term if needed.
        if self.current_term != msg.term {
            self.update_current_term(msg.term, None);
            self.save_hard_state(ctx);
        }

        // Update current leader if needed.
        if self.current_leader != Some(msg.leader_id) {
            self.update_current_leader(ctx, UpdateCurrentLeader::OtherNode(msg.leader_id));
        }

        // If not follower, become follower.
        if !self.state.is_follower() && !self.state.is_non_voter() {
            self.become_follower(ctx);
        }

        // Extract follower specific state.
        let state = match &mut self.state {
            RaftState::Follower(state) => state,
            _ => return Box::new(fut::err(())),
        };

        // Compare current snapshot state with received RPC and handle as needed.
        match &mut state.snapshot_state {
            // Install a new snapshot which was small enough to fit into a single frame.
            SnapshotState::Idle if msg.done => self.handle_mini_snapshot(ctx, msg),
            // Begin streaming in & installing a new snapshot.
            SnapshotState::Idle => self.handle_snapshot_stream(ctx, msg),
            SnapshotState::Streaming(txopt, finalrxopt) if msg.done => {
                // Done streaming in the snapshot, send final message and transition to Idle state.
                if let (Some(tx), Some(finalrx)) = (txopt.take(), finalrxopt.take()) {
                    self.handle_final_snapshot_chunk(ctx, msg, tx, finalrx)
                } else {
                    // Duplicate message after one of the channels has been dropped. Err and return to Idle.
                    state.snapshot_state = SnapshotState::Idle;
                    Box::new(fut::err(()))
                }
            }
            // Pipe a new snapshot chunk through the stream.
            SnapshotState::Streaming(Some(tx), _) => {
                let tx = tx.clone();
                self.handle_snapshot_chunk(ctx, msg, tx.clone())
            },
            // Duplicate message after one of the channels has been dropped. Err and return to Idle.
            SnapshotState::Streaming(_, _) => Box::new(fut::err(())),
        }
    }
}

impl<D: AppData, E: AppError, N: RaftNetwork<D>, S: RaftStorage<D, E>> Raft<D, E, N, S> {
    // Install a new snapshot which was small enough to fit into a single frame.
    fn handle_mini_snapshot(&mut self, ctx: &mut Context<Self>, msg: InstallSnapshotRequest) -> Box<dyn ActorFuture<Actor=Self, Item=InstallSnapshotResponse, Error=()>> {
        let (tx, rx) = mpsc::unbounded();
        let (chunktx, chunkrx) = oneshot::channel();
        let (finaltx, finalrx) = oneshot::channel();

        // Start storage engine task.
        let (snap_index, snap_term) = (msg.last_included_index, msg.last_included_term);
        let task = fut::wrap_future(self.storage.send(InstallSnapshot::new(snap_term, snap_index, rx)))
            .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftStorage))
            .and_then(|res, act, ctx| act.map_fatal_storage_result(ctx, res))
            .map(move |_, _, _| {
                // This will be called after all snapshot chunks have been streamed in and
                // we've received the final response from the storage engine.
                let _ = finaltx.send(());
            });
        ctx.spawn(task);

        // Send first & final chunk of data.
        match tx.unbounded_send(InstallSnapshotChunk{offset: msg.offset, data: msg.data, done: msg.done, cb: chunktx}) {
            Ok(_) => (),
            Err(_) => {
                error!("Error streaming snapshot chunks to storage engine. Channel was closed.");
                if let RaftState::Follower(state) = &mut self.state {
                    state.snapshot_state = SnapshotState::Idle;
                }
                return Box::new(fut::err(()));
            }
        };
        Box::new(fut::wrap_future(chunkrx)
            .and_then(|_, _, _| fut::wrap_future(finalrx))
            .then(move |res, act: &mut Self, _| match res {
                Ok(_) => match &mut act.state {
                    RaftState::Follower(state) => {
                        debug!("Finished installing snapshot. Update index & term to {} & {}.", snap_index, snap_term);
                        state.snapshot_state = SnapshotState::Idle;
                        if act.last_log_index < snap_index {
                            act.last_log_index = snap_index;
                            act.last_log_term = snap_term;
                            act.last_applied = snap_index;
                        }
                        fut::ok(InstallSnapshotResponse{term: act.current_term})
                    }
                    _ => fut::err(()),
                }
                Err(_) => {
                    error!("Error awaiting response from storage engine for final snapshot chunk. Channel was closed.");
                    fut::err(())
                }
            }))
    }

    fn handle_snapshot_stream(&mut self, ctx: &mut Context<Self>, msg: InstallSnapshotRequest) -> Box<dyn ActorFuture<Actor=Self, Item=InstallSnapshotResponse, Error=()>> {
        let (tx, rx) = mpsc::unbounded();
        let (chunktx, chunkrx) = oneshot::channel();
        let (finaltx, finalrx) = oneshot::channel();
        match &mut self.state {
            RaftState::Follower(state) => {
                state.snapshot_state = SnapshotState::Streaming(Some(tx.clone()), Some(finalrx));
            }
            _ => return Box::new(fut::err(())),
        }

        let (snap_index, snap_term) = (msg.last_included_index, msg.last_included_term);
        let f = fut::wrap_future(self.storage.send(InstallSnapshot::new(snap_term, snap_index, rx)))
            .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftStorage))
            .and_then(|res, act, ctx| act.map_fatal_storage_result(ctx, res))
            .map(move |_, _, _| {
                debug!("Received final response from storage engine for snapshot stream.");
                // This will be called after all snapshot chunks have been streamed in and
                // we've received the final response from the storage engine.
                let _ = finaltx.send(());
            });
        ctx.spawn(f);

        match tx.unbounded_send(InstallSnapshotChunk{offset: msg.offset, data: msg.data, done: msg.done, cb: chunktx}) {
            Ok(_) => (),
            Err(_) => {
                error!("Error streaming snapshot chunks to storage engine. Channel was closed.");
                if let RaftState::Follower(state) = &mut self.state {
                    state.snapshot_state = SnapshotState::Idle;
                }
                return Box::new(fut::err(()));
            }
        };
        Box::new(fut::wrap_future(chunkrx)
            .then(|res, act: &mut Self, _| match res {
                Ok(_) => fut::ok(InstallSnapshotResponse{term: act.current_term}),
                Err(_) => {
                    error!("Error awaiting response from storage engine for chunk response. Channel was closed.");
                    fut::err(())
                }
            }))
    }

    fn handle_final_snapshot_chunk(
        &mut self, _: &mut Context<Self>, msg: InstallSnapshotRequest, tx: mpsc::UnboundedSender<InstallSnapshotChunk>, finalrx: oneshot::Receiver<()>,
    ) -> Box<dyn ActorFuture<Actor=Self, Item=InstallSnapshotResponse, Error=()>> {
        let (chunktx, chunkrx) = oneshot::channel();
        let (snap_index, snap_term) = (msg.last_included_index, msg.last_included_term);
        match tx.unbounded_send(InstallSnapshotChunk{offset: msg.offset, data: msg.data, done: msg.done, cb: chunktx}) {
            Ok(_) => (),
            Err(_) => {
                error!("Error streaming snapshot chunks for storage engine. Channel was closed.");
                if let RaftState::Follower(state) = &mut self.state {
                    state.snapshot_state = SnapshotState::Idle;
                }
                return Box::new(fut::err(()));
            }
        };
        Box::new(fut::wrap_future(chunkrx)
            .and_then(|_, _, _| fut::wrap_future(finalrx))
            .then(move |res, act: &mut Self, _| match res {
                Ok(_) => match &mut act.state {
                    RaftState::Follower(state) => {
                        debug!("Finished installing snapshot. Update index & term to {} & {}.", snap_index, snap_term);
                        state.snapshot_state = SnapshotState::Idle;
                        if act.last_log_index < snap_index {
                            act.last_log_index = snap_index;
                            act.last_log_term = snap_term;
                            act.last_applied = snap_index;
                        }
                        fut::ok(InstallSnapshotResponse{term: act.current_term})
                    }
                    _ => fut::err(()),
                }
                Err(_) => {
                    error!("Error awaiting response from storage engine for final snapshot chunk. Channel was closed.");
                    fut::err(())
                }
            }))
    }

    fn handle_snapshot_chunk(
        &mut self, _: &mut Context<Self>, msg: InstallSnapshotRequest, tx: mpsc::UnboundedSender<InstallSnapshotChunk>,
    ) -> Box<dyn ActorFuture<Actor=Self, Item=InstallSnapshotResponse, Error=()>> {
        let (chunktx, chunkrx) = oneshot::channel();
        match tx.unbounded_send(InstallSnapshotChunk{offset: msg.offset, data: msg.data, done: msg.done, cb: chunktx}) {
            Ok(_) => (),
            Err(_) => {
                error!("Error streaming snapshot chunks to storage engine. Channel was closed.");
                if let RaftState::Follower(state) = &mut self.state {
                    state.snapshot_state = SnapshotState::Idle;
                }
                return Box::new(fut::err(()));
            }
        };
        Box::new(fut::wrap_future(chunkrx)
            .then(|res, act: &mut Self, _| match res {
                Ok(_) => fut::ok(InstallSnapshotResponse{term: act.current_term}),
                Err(_) => {
                    error!("Node {}: awaiting response from storage engine for chunk response. Channel was closed.", act.id);
                    fut::err(())
                }
            }))
    }
}
