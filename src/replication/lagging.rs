use std::time::{Duration, Instant};

use actix::prelude::*;
use log::{debug};
use tokio_timer::Delay;

use crate::{
    AppData, AppError,
    common::DependencyAddr,
    config::SnapshotPolicy,
    messages::{AppendEntriesRequest},
    network::RaftNetwork,
    replication::{ReplicationStream, RSRateUpdate, RSState},
    storage::{RaftStorage, GetLogEntries},
};

impl<D: AppData, E: AppError, N: RaftNetwork<D>, S: RaftStorage<D, E>> ReplicationStream<D, E, N, S> {
    /// Drive the replication stream forward when it is in state `Lagging`.
    pub(super) fn drive_state_lagging(&mut self, ctx: &mut Context<Self>) {
        let state = match &mut self.state {
            RSState::Lagging(state) => state,
            _ => {
                self.is_driving_state = false;
                return self.drive_state(ctx);
            },
        };

        // A few values to be moved into future closures.
        let (prev_log_index, prev_log_term) = (self.match_index, self.match_term);
        let start = self.next_index;
        let batch_will_reach_line = (self.next_index > self.line_index) || ((self.line_index - self.next_index) < self.config.max_payload_entries);

        // Do a preliminary check to see if we need to transition over to snapshotting state,
        // which may come about due to a node returning lots of errors or dropping lots of frames.
        if let SnapshotPolicy::LogsSinceLast(threshold) = &self.config.snapshot_policy {
            if self.line_index > self.match_index && (self.line_index - self.match_index) >= *threshold {
                debug!("{} sees {} as too far behind. Needs snapshot.", self.id, self.target);
                let f = self.transition_to_snapshotting(ctx)
                    .and_then(|_, act, ctx| {
                        act.is_driving_state = false;
                        act.drive_state(ctx);
                        fut::ok(())
                    });
                ctx.spawn(f);
                return;
            }
        }

        // Determine an appropriate stop index for the storage fetch operation. Avoid underflow.
        ctx.spawn(
            (if batch_will_reach_line {
                // If we have caught up to the line index, then that means we will be running at
                // line rate after this payload is successfully replicated.
                let stop_idx = self.line_index + 1; // Fetch operation is non-inclusive on the stop value, so ensure it is included.
                state.is_ready_for_line_rate = true;

                // Update Raft actor with replication rate change.
                let event = RSRateUpdate{target: self.target, is_line_rate: true};
                fut::Either::A(fut::wrap_future(self.raftnode.send(event))
                    .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftInternal))
                    .map(move |_, _, _| stop_idx))
            } else {
                fut::Either::B(fut::ok(self.next_index + self.config.max_payload_entries))
            })

            // Bringing the target up-to-date by fetching the largest possible payload of entries
            // from storage within permitted configuration.
            .and_then(move |stop, act: &mut Self, _| {
                fut::wrap_future(act.storage.send(GetLogEntries::new(start, stop)))
                    .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftStorage))
            })
            .and_then(|res, act, ctx| act.map_fatal_storage_result(ctx, res))

            // We have a successful payload of entries, send it to the target.
            .and_then(move |entries, act, ctx| {
                let last_log_and_index = entries.last().map(|elem| (elem.index, elem.term));
                let payload = AppendEntriesRequest{
                    target: act.target, term: act.term, leader_id: act.id,
                    prev_log_index, prev_log_term, // NOTE: these are moved in from above.
                    entries, leader_commit: act.line_commit,
                };
                act.send_append_entries(ctx, payload)
                    .and_then(move |res, act, ctx| act.handle_append_entries_response(ctx, res, last_log_and_index))
            })

            // Transition to line rate if needed.
            .and_then(|_, act, ctx| {
                match &act.state {
                    RSState::Lagging(inner) if inner.is_ready_for_line_rate => {
                        fut::Either::A(act.transition_to_line_rate(ctx))
                    }
                    _ => fut::Either::B(fut::ok(())),
                }
            })

            // If an error has come up during this workflow, rate limit the next iteration.
            .then(|res, _, _| match res {
                Ok(ok) => fut::Either::A(fut::ok(ok)),
                Err(err) => {
                    let delay = Instant::now() + Duration::from_secs(1);
                    fut::Either::B(fut::wrap_future(Delay::new(delay).map_err(|_| ()).then(move |res| match res {
                        Ok(_) => Err(err),
                        Err(_) => Err(err),
                    })))
                }
            })

            // Drive state forward regardless of outcome.
            .then(|res, act, ctx| {
                act.is_driving_state = false;
                act.drive_state(ctx);
                fut::result(res)
            }));
    }
}
