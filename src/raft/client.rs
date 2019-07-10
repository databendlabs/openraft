use std::sync::Arc;

use actix::prelude::*;
use log::{error, warn};
use futures::sync::oneshot;

use crate::{
    AppError,
    network::RaftNetwork,
    messages::{
        ClientError, ClientPayload, ClientPayloadResponse,
        Entry, EntryType, ResponseMode,
    },
    raft::{RaftState, Raft, common::{AwaitingCommitted, ClientPayloadWithTx, DependencyAddr}},
    replication::RSReplicate,
    storage::{AppendLogEntries, ApplyEntriesToStateMachine, RaftStorage},
};

const CLIENT_RPC_CHAN_ERR: &str = "Client RPC channel was unexpectedly closed.";

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
            .map_err(|_, _, _| {
                error!("Internal client response channel was unexpectedly dropped.");
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
            let ClientPayloadWithTx{tx, rpc: mut rpc_out} = msg;
            rpc_out.target = *target;
            let f = fut::wrap_future(self.network.send(rpc_out))
                .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftNetwork))
                .then(move |res, _, _| match res {
                    Ok(inner) => {
                        let _ = tx.send(inner).map_err(|_| ());
                        fut::ok(())
                    }
                    Err(_) => {
                        // Here, we will already be shutting down, but just return ok.
                        fut::ok(())
                    }
                });
            ctx.spawn(f);
        } else {
            // We don't know the ID of the current leader, or one has not been elected yet. Buffer the request.
            self.forwarding.push(msg);
        }
    }


    /// Process the given client RPC, appending it to the log and issuing the needed response.
    ///
    /// This function takes the given RPC, appends its entries to the log, sends the entries out
    /// to the replication streams to be replicated to the cluster followers, after half of the
    /// cluster members have successfully replicated the entries this routine will proceed with
    /// applying the entries to the state machine. Then the next RPC is processed.
    ///
    /// TODO: there is an optimization to be had here:
    /// - after half of the nodes have replicated the entries of the RPC, we can technically begin
    /// processing the next RPC.
    /// - applying entries to the state machine must still be done in order, but we can create a
    /// buffer of entries which will ensure entries are applied in serial order.
    /// - if we use this approach, the RPC along with its entries could be buffered together so
    /// that after the RPC's segment of entries have been applied, responses can be issued for the
    /// RPCs which were submitted with `ResponseMode::Applied`.
    ///
    /// TODO: this algorithm needs to be updated to use an Arc<_> instead of making so many clones
    /// of the entries payload.
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
        let mut line_index = self.last_log_index;
        let entries: Arc<Vec<_>> = Arc::new(msg.rpc.entries.clone().into_iter().map(|data| {
            line_index += 1;
            Entry{
                index: line_index,
                term: self.current_term,
                entry_type: EntryType::Normal(data),
            }
        }).collect());

        // Send the entries over to the storage engine.
        self.is_appending_logs = true;
        let f = fut::wrap_future(self.storage.send(AppendLogEntries::new(entries.clone())))
            .map_err(|err, act: &mut Self, ctx| {
                act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftStorage);
                ClientError::Internal
            })
            .and_then(|res, _, _| fut::result(res.map_err(|err| ClientError::Application(err))))
            // Handle results from storage engine.
            .then(move |res, act, _| match res {
                Ok(_) => {
                    act.last_log_index = line_index;
                    act.is_appending_logs = false;
                    fut::result(Ok((msg, entries)))
                }
                Err(err) => {
                    let _ = msg.tx.send(Err(err)).map_err(|err| error!("{} {:?}", CLIENT_RPC_CHAN_ERR, err));
                    fut::err(())
                }
            })

            // Send logs over for replication.
            .and_then(move |(rpc, entries), act, ctx| {
                // Get a reference to the leader's state, else forward to leader.
                let (state, rx) = match &mut act.state {
                    RaftState::Leader(state) => {
                        let (tx, rx) = oneshot::channel();
                        state.awaiting_committed = Some(AwaitingCommitted{index: line_index, rpc, chan: tx});
                        (state, rx)
                    },
                    _ => {
                        act.forward_request_to_leader(ctx, rpc);
                        return fut::Either::A(fut::err(()));
                    }
                };

                // Send payload over to each replication stream as needed.
                for rs in state.nodes.values() {
                    // Only send full payload over if the target stream is running at line rate.
                    let _ = rs.addr.do_send(RSReplicate{entries: entries.clone(), line_index, line_commit: act.commit_index});
                }

                // Resolve this step in the pipeline once the RPC's entries have been comitted accross the cluster.
                fut::Either::B(fut::wrap_future(rx
                    .map(|val| (val, entries))
                    .map_err(|_| ())))
            })
            // The RPC's entries have been committed, handle client response & applying to state machine locally.
            .and_then(move |(rpc, entries), act, _| {
                // If this RPC is configured to wait only for log committed, then respond to client now.
                let tx = if let &ResponseMode::Committed = &rpc.rpc.response_mode {
                    let client_response = ClientPayloadResponse{indices: entries.iter().map(|e| e.index).collect()};
                    let _ = rpc.tx.send(Ok(client_response)).map_err(|err| error!("{} {:?}", CLIENT_RPC_CHAN_ERR, err));
                    None
                } else {
                    Some(rpc.tx)
                };

                // Apply entries to state machine.
                act.is_applying_logs_to_state_machine = true;
                fut::wrap_future(act.storage.send(ApplyEntriesToStateMachine::new(entries.clone())))
                    .map_err(|err, act: &mut Self, ctx| {
                        act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftStorage);
                        ClientError::Internal
                    })
                    .and_then(|res, act, ctx| act.map_fatal_storage_result(ctx, res)
                        .map_err(|_, _, _| ClientError::Internal))

                    .then(move |res, act, _| match res {
                        Ok(_data) => {
                            // Update state after a success operation on the state machine.
                            act.is_applying_logs_to_state_machine = false;
                            act.last_applied = line_index;

                            // If this RPC is configured to wait for applied, then respond to client now.
                            if let Some(tx) = tx {
                                let client_response = ClientPayloadResponse{indices: entries.iter().map(|e| e.index).collect()};
                                let _ = tx.send(Ok(client_response)).map_err(|err| error!("{} {:?}", CLIENT_RPC_CHAN_ERR, err));
                            }
                            fut::ok(())
                        }
                        Err(err) => {
                            if let Some(tx) = tx {
                                let _ = tx.send(Err(err)).map_err(|err| error!("{} {:?}", CLIENT_RPC_CHAN_ERR, err));
                            }
                            fut::err(())
                        }
                    })
            });
        fut::Either::B(f)
    }
}
