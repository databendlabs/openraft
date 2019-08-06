use actix::prelude::*;

use crate::{
    AppError,
    common::DependencyAddr,
    messages::{
        AppendEntriesRequest, AppendEntriesResponse,
    },
    network::RaftNetwork,
    replication::{ReplicationStream, RSRevertToFollower},
    storage::{RaftStorage},
};

impl<E: AppError, N: RaftNetwork<E>, S: RaftStorage<E>> ReplicationStream<E, N, S> {

    /// Handle heartbeat responses.
    ///
    /// For heartbeat responses, we really only care about checking for more recent terms. We
    /// don't do any conflict resolution or anything like that with heartbeats.
    fn handle_heartbeat_response(&mut self, _: &mut Context<Self>, res: AppendEntriesResponse) -> impl ActorFuture<Actor=Self, Item=(), Error=()> {
        // Replication was not successful, if a newer term has been returned, revert to follower.
        if &res.term > &self.term {
            fut::Either::A(fut::wrap_future(self.raftnode.send(RSRevertToFollower{target: self.target, term: res.term}))
                .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftInternal))
                .then(|res, _, ctx| {
                    ctx.terminate(); // Terminate this replication stream.
                    fut::result(res)
                }))
        } else {
            fut::Either::B(fut::ok(()))
        }
    }

    /// Setup the heartbeat mechanism.
    pub(super) fn setup_heartbeat(&mut self, ctx: &mut Context<Self>) {
        // Setup a new heartbeat to be sent after the `heartbeat_rate` duration.
        let duration = std::time::Duration::from_millis(self.config.heartbeat_interval);
        ctx.run_interval(duration, |act, ctx| {
            let f = act.heartbeat_send(ctx);
            ctx.spawn(f);
        });
    }

    /// Send a heartbeat frame to the target node.
    pub(super) fn heartbeat_send(&mut self, _: &mut Context<Self>) -> impl ActorFuture<Actor=Self, Item=(), Error=()> {
        // Build the heartbeat frame to be sent to the follower.
        let payload = AppendEntriesRequest{
            target: self.target, term: self.term, leader_id: self.id,
            prev_log_index: self.match_index, prev_log_term: self.match_term,
            entries: Vec::with_capacity(0), leader_commit: self.line_commit,
        };

        // Send the payload.
        fut::wrap_future(self.network.send(payload))
            .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftNetwork))
            .and_then(|res, _, _| fut::result(res))
            .and_then(|res, act, ctx| act.handle_heartbeat_response(ctx, res))
    }
}
