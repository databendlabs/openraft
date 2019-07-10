use actix::prelude::*;

use crate::{
    AppError,
    network::RaftNetwork,
    raft::{Raft, common::UpdateCurrentLeader},
    replication::{
        RSNeedsSnapshot, RSNeedsSnapshotResponse,
        RSRateUpdate, RSRevertToFollower, RSUpdateMatchIndex,
    },
    storage::RaftStorage,
};

//////////////////////////////////////////////////////////////////////////////////////////////////
// RSRateUpdate //////////////////////////////////////////////////////////////////////////////////

impl<E: AppError, N: RaftNetwork<E>, S: RaftStorage<E>> Handler<RSRateUpdate> for Raft<E, N, S> {
    type Result = ();

    /// Handle events from replication streams.
    ///
    /// TODO: finish this up.
    fn handle(&mut self, _msg: RSRateUpdate, _ctx: &mut Self::Context) {
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// RSNeedsSnapshot ///////////////////////////////////////////////////////////////////////////////

impl<E: AppError, N: RaftNetwork<E>, S: RaftStorage<E>> Handler<RSNeedsSnapshot> for Raft<E, N, S> {
    type Result = ResponseActFuture<Self, RSNeedsSnapshotResponse, ()>;

    /// Handle events from replication streams requesting for snapshot info.
    ///
    /// TODO: finish this up.
    fn handle(&mut self, _msg: RSNeedsSnapshot, _ctx: &mut Self::Context) -> Self::Result {
        Box::new(fut::err(()))
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// RSRevertToFollower ////////////////////////////////////////////////////////////////////////////

impl<E: AppError, N: RaftNetwork<E>, S: RaftStorage<E>> Handler<RSRevertToFollower> for Raft<E, N, S> {
    type Result = ();

    /// Handle events from replication streams for when this node needs to revert to follower state.
    ///
    /// TODO: finish this up.
    fn handle(&mut self, _msg: RSRevertToFollower, ctx: &mut Self::Context) {
        self.update_current_leader(ctx, UpdateCurrentLeader::Unknown);
        self.become_follower(ctx);
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// RSUpdateMatchIndex ////////////////////////////////////////////////////////////////////////////

impl<E: AppError, N: RaftNetwork<E>, S: RaftStorage<E>> Handler<RSUpdateMatchIndex> for Raft<E, N, S> {
    type Result = ();

    /// Handle events from a replication stream which updates the target node's match index.
    ///
    /// TODO: finish this up.
    fn handle(&mut self, _msg: RSUpdateMatchIndex, _ctx: &mut Self::Context) {
    }
}
