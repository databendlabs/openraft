
use actix::prelude::*;

use crate::{
    AppData, AppDataResponse, AppError,
    messages::{AppendEntriesRequest},
    network::RaftNetwork,
    replication::{ReplicationStream, RSState},
    storage::{RaftStorage},
};

impl<D: AppData, R: AppDataResponse, E: AppError, N: RaftNetwork<D>, S: RaftStorage<D, R, E>> ReplicationStream<D, R, E, N, S> {
    /// Drive the replication stream forward when it is in state `LineRate`.
    pub(super) fn drive_state_line_rate(&mut self, ctx: &mut Context<Self>) {
        let state = match &mut self.state {
            RSState::LineRate(state) => state,
            _ => {
                self.is_driving_state = false;
                return self.drive_state(ctx);
            },
        };

        // If there is a buffered payload, send it, else nothing to do.
        if state.buffered_outbound.len() > 0 {
            let entries: Vec<_> = state.buffered_outbound.drain(..).map(|elem| (*elem).clone()).collect();
            let last_index_and_term = entries.last().map(|e| (e.index, e.term));
            let payload = AppendEntriesRequest{
                target: self.target, term: self.term, leader_id: self.id,
                prev_log_index: self.match_index,
                prev_log_term: self.match_term,
                entries, leader_commit: self.line_commit,
            };

            // Send the payload.
            let f = self.send_append_entries(ctx, payload)
                // Process the response.
                .and_then(move |res, act, ctx| act.handle_append_entries_response(ctx, res, last_index_and_term))

                // Drive state forward regardless of outcome.
                .then(|res, act, ctx| {
                    act.is_driving_state = false;
                    match res {
                        Ok(_) => {
                            act.drive_state(ctx);
                            fut::Either::A(fut::result(res))
                        }
                        Err(_) => {
                            fut::Either::B(act.transition_to_lagging(ctx)
                                .then(|res, act, ctx| {
                                    act.drive_state(ctx);
                                    fut::result(res)
                                }))
                        }
                    }
                });
            ctx.spawn(f);
        } else {
            self.is_driving_state = false;
        }
    }
}
