use actix::prelude::*;
use log::{debug, error, warn};
use futures::sync::oneshot;

use crate::{
    AppError,
    common::{CLIENT_RPC_CHAN_ERR, ClientPayloadUpgraded, ClientPayloadWithTx, DependencyAddr},
    network::RaftNetwork,
    messages::{ClientError, ClientPayload, ClientPayloadForwarded, ClientPayloadResponse},
    raft::{RaftState, Raft},
    replication::RSReplicate,
    storage::{AppendLogEntries, AppendLogEntriesMode, RaftStorage},
};

impl<E: AppError, N: RaftNetwork<E>, S: RaftStorage<E>> Handler<ClientPayload<E>> for Raft<E, N, S> {
    type Result = ResponseActFuture<Self, ClientPayloadResponse, ClientError<E>>;

    /// Handle client requests.
    fn handle(&mut self, msg: ClientPayload<E>, ctx: &mut Self::Context) -> Self::Result {
        // Only handle requests if actor has finished initialization.
        if let &RaftState::Initializing = &self.state {
            warn!("Received Raft RPC before initialization was complete.");
            return Box::new(fut::err(ClientError::Internal));
        }

        // Wrap the given message for async processing.
        let (tx, rx) = oneshot::channel();
        let with_tx = ClientPayloadWithTx{tx, rpc: msg};

        // Queue the message for processing or forward it along to the leader.
        match &mut self.state {
            RaftState::Leader(state) => {
                let _ = state.client_request_queue.unbounded_send(with_tx).map_err(|_| {
                    error!("Unexpected error while queueing client request for processing.")
                });
            },
            _ => self.forward_request_to_leader(ctx, with_tx),
        };

        // Build a response from the message's channel.
        Box::new(fut::wrap_future(rx)
            .map_err(|_, act: &mut Self, _| {
                error!("Node {} client request: internal client response channel was unexpectedly dropped.", &act.id);
                ClientError::Internal
            })
            .and_then(|res, _, _| fut::result(res)))
    }
}

impl<E: AppError, N: RaftNetwork<E>, S: RaftStorage<E>> Raft<E, N, S> {
    /// Forward the given client request to the last known leader of the cluster.
    fn forward_request_to_leader(&mut self, ctx: &mut Context<Self>, msg: ClientPayloadWithTx<E>) {
        // If we have a currently known leader, then forward the request to it.
        if let Some(target) = &self.current_leader {
            let ClientPayloadWithTx{tx, rpc: payload} = msg;
            let outbound = ClientPayloadForwarded{target: *target, from: self.id, payload};
            let f = fut::wrap_future(self.network.send(outbound))
                .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftNetwork))
                .then(move |res, act, _| match res {
                    Ok(inner) => {
                        let _ = tx.send(inner).map_err(|_| ());
                        fut::ok(())
                    }
                    Err(_) => {
                        error!("Node {} is dropping client request due to actix messaging error.", &act.id);
                        // Here, we will already be shutting down, but just return ok.
                        fut::ok(())
                    }
                });
            ctx.spawn(f);
        } else {
            // We don't know the ID of the current leader, or one has not been elected yet. Buffer the request.
            debug!("Node {} is buffering client request until leader is known.", &self.id);
            self.forwarding.push(msg);
        }
    }


    /// Process the given client RPC, appending it to the log and committing it to the cluster.
    ///
    /// This function takes the given RPC, appends its entries to the log, sends the entries out
    /// to the replication streams to be replicated to the cluster followers, after half of the
    /// cluster members have successfully replicated the entries this routine will proceed with
    /// applying the entries to the state machine. Then the next RPC is processed.
    pub(super) fn process_client_rpc(&mut self, ctx: &mut Context<Self>, msg: ClientPayloadWithTx<E>) -> impl ActorFuture<Actor=Self, Item=(), Error=()> {
        match &self.state {
            // If node is still leader, continue.
            RaftState::Leader(_) => (),
            // If node is in any other state, then forward the message to the leader.
            _ => {
                self.forward_request_to_leader(ctx, msg);
                return fut::Either::A(fut::ok(()));
            }
        };

        // Transform the entries of the RPC into log entries.
        let payload = ClientPayloadUpgraded::upgrade(msg, self.last_log_index, self.current_term);

        // Send the entries over to the storage engine.
        self.is_appending_logs = true; // NOTE: this routine is pipelined, but we still use a semaphore in case of transition to follower.
        fut::Either::B(fut::wrap_future(self.storage.send(AppendLogEntries::new(AppendLogEntriesMode::Leader, payload.entries())))
            .map_err(|err, act: &mut Self, ctx| {
                act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftStorage);
                ClientError::Internal
            })
            .and_then(|res, _, _| fut::result(res.map_err(|err| ClientError::Application(err))))

            // Handle results from storage engine.
            .then(move |res, act, _| {
                act.is_appending_logs = false;
                match res {
                    Ok(_) => {
                        act.last_log_index = payload.last_index;
                        act.last_log_term = act.current_term;
                        fut::result(Ok(payload))
                    }
                    Err(err) => {
                        debug!("Node {} received an error from the storage engine.", &act.id);
                        let _ = payload.tx.send(Err(err)).map_err(|err| error!("{} {:?}", CLIENT_RPC_CHAN_ERR, err));
                        fut::err(())
                    }
                }
            })

            // Send logs over for replication.
            .and_then(move |payload, act, ctx| {
                match &mut act.state {
                    RaftState::Leader(state) => {
                        // Setup the request to await for being committed to the cluster.
                        let entries = payload.entries();
                        let line_index = payload.last_index;
                        state.awaiting_committed.push(payload);

                        // Send payload over to each replication stream as needed.
                        for rs in state.nodes.values() {
                            // Only send full payload over if the target stream is running at line rate.
                            let _ = rs.addr.do_send(RSReplicate{entries: entries.clone(), line_index, line_commit: act.commit_index});
                        }
                    },
                    _ => {
                        let msg = payload.downgrade();
                        act.forward_request_to_leader(ctx, msg);
                    }
                }
                fut::ok(())
            }))
    }
}
