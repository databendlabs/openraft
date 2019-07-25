use std::sync::Arc;

use actix::prelude::*;
use futures::sync::oneshot;
use log::{error, debug};

use crate::{
    AppError,
    common::{CLIENT_RPC_CHAN_ERR, ApplyLogsTask, ClientPayloadUpgraded, DependencyAddr},
    messages::{ClientPayloadResponse, ClientError, Entry, ResponseMode},
    network::RaftNetwork,
    raft::Raft,
    storage::{ApplyEntriesToStateMachine, GetLogEntries, RaftStorage},
};

impl<E: AppError, N: RaftNetwork<E>, S: RaftStorage<E>> Raft<E, N, S> {
    /// Process tasks for applying logs to the state machine.
    ///
    /// **NOTE WELL:** these operations are strictly pipelined to ensure that these operations
    /// happen in strict order for guaranteed linearizability.
    pub(super) fn process_apply_logs_task(&mut self, ctx: &mut Context<Self>, msg: ApplyLogsTask<E>) -> impl ActorFuture<Actor=Self, Item=(), Error=()> {
        match msg {
            ApplyLogsTask::Entries{entries, res} => fut::Either::A(self.process_apply_logs_task_with_entries(ctx, entries, res)),
            ApplyLogsTask::Outstanding => fut::Either::B(self.process_apply_logs_task_outstanding(ctx)),
        }
    }

    /// Apply the given payload of log entries to the state machine.
    fn process_apply_logs_task_with_entries(
        &mut self, _: &mut Context<Self>, entries: Arc<Vec<Entry>>, chan: Option<oneshot::Sender<Result<ClientPayloadResponse, ClientError<E>>>>,
    ) -> impl ActorFuture<Actor=Self, Item=(), Error=()> {
        // PREVIOUS TERM UNCOMMITTED LOGS CHECK:
        // Here we are checking to see if there are any logs from the previous term which were
        // outstanding, but which are now safe to apply due to being covered by a new definitive
        // commit index from this term.
        let f = if let Some(first) = entries.iter().nth(0).map(|e| e.index) {
            // There are uncommited logs from last term which are now considered committed and are safe to apply.
            if (self.last_applied + 1) != first {
                fut::Either::A(fut::wrap_future(self.storage.send(GetLogEntries::new(self.last_applied + 1, first)))
                    .map_err(|err, act: &mut Self, ctx| {
                        act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftStorage);
                        ClientError::Internal
                    })
                    .and_then(|res, act, ctx| act.map_fatal_storage_result(ctx, res).map_err(|_, _, _| ClientError::Internal))
                    .and_then(|res, act, _| {
                        let old_entries = Arc::new(res);
                        fut::wrap_future(act.storage.send(ApplyEntriesToStateMachine::new(old_entries.clone())))
                            .map_err(|err, act: &mut Self, ctx| {
                                act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftStorage);
                                ClientError::Internal
                            })
                            .and_then(|res, act, ctx| act.map_fatal_storage_result(ctx, res).map_err(|_, _, _| ClientError::Internal))
                            .and_then(move |_, act, _| {
                                // Update state after a success operation on the state machine.
                                if let Some(line_index) = old_entries.iter().last().map(|e| e.index) {
                                    debug!("Node {} has applied index {} from previous term to state machine.", &act.id, &line_index);
                                    act.last_applied = line_index;
                                }
                                fut::ok(())
                            })
                    }))
            } else {
                let res: Result<(), ClientError<E>> = Ok(());
                fut::Either::B(fut::result(res))
            }
        } else {
            let res: Result<(), ClientError<E>> = Ok(());
            fut::Either::B(fut::result(res))
        };

        // Resume with the normal flow of logic. Here we are simply taking the payload of entries
        // to be applied to the state machine, applying and then responding as needed.
        let entries_clone = entries.clone();
        f.and_then(move |_, act, _| fut::wrap_future(act.storage.send(ApplyEntriesToStateMachine::new(entries_clone)))
            .map_err(|err, act: &mut Self, ctx| {
                act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftStorage);
                ClientError::Internal
            })
            .and_then(|res, act, ctx| act.map_fatal_storage_result(ctx, res)
                .map_err(|_, _, _| ClientError::Internal)))

            .then(move |res, act, _| match res {
                Ok(_data) => {
                    // Update state after a success operation on the state machine.
                    if let Some(line_index) = entries.iter().last().map(|e| e.index) {
                        debug!("Node {} has applied index {} to state machine.", &act.id, &line_index);
                        act.last_applied = line_index;
                    }

                    // If this RPC is configured to wait for applied, then respond to client now.
                    if let Some(tx) = chan {
                        let client_response = ClientPayloadResponse{indices: entries.iter().map(|e| e.index).collect()};
                        let _ = tx.send(Ok(client_response)).map_err(|err| error!("{} {:?}", CLIENT_RPC_CHAN_ERR, err));
                    }
                    fut::ok(())
                }
                Err(err) => {
                    if let Some(tx) = chan {
                        let _ = tx.send(Err(err)).map_err(|err| error!("{} {:?}", CLIENT_RPC_CHAN_ERR, err));
                    }
                    fut::err(())
                }
            })
    }

    /// Check for any outstanding logs which have been committed but which have not been applied,
    /// then apply them.
    fn process_apply_logs_task_outstanding(&mut self, _: &mut Context<Self>) -> impl ActorFuture<Actor=Self, Item=(), Error=()> {
        // Guard against no-op.
        if &self.last_applied == &self.commit_index {
            return fut::Either::A(fut::ok(()));
        }

        // Fetch the series of entries which must be applied to the state machine.
        let start = self.last_applied + 1;
        let stop = self.commit_index + 1;
        fut::Either::B(fut::wrap_future(self.storage.send(GetLogEntries::new(start, stop)))
            .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftStorage))
            .and_then(|res, act, ctx| act.map_fatal_storage_result(ctx, res))

            // Send the entries over to the storage engine to be applied to the state machine.
            .and_then(|entries, act, _| {
                let entries = Arc::new(entries);
                fut::wrap_future(act.storage.send(ApplyEntriesToStateMachine::new(entries.clone())))
                    .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftStorage))
                    .and_then(|res, act, ctx| act.map_fatal_storage_result(ctx, res))
                    .map(move |_, _, _| entries)
            })

            // Update self to reflect progress on applying logs to the state machine.
            .and_then(move |entries, act, _| {
                if let Some(idx) = entries.last().map(|elem| elem.index) {
                    debug!("Node {} has replicated index {} to state machine.", &act.id, &idx);
                    act.last_applied = idx;
                }
                fut::ok(())
            }))
    }

    /// Process the final stage of a client request after it has been committed to the cluster.
    pub(super) fn process_committed_client_request(&mut self, _ctx: &mut Context<Self>, payload: ClientPayloadUpgraded<E>) -> impl ActorFuture<Actor=Self, Item=(), Error=()> {
        // If this RPC is configured to wait only for log committed, then respond to client now.
        let task = if let &ResponseMode::Committed = &payload.response_mode {
            let entries = payload.entries();
            let client_response = ClientPayloadResponse{indices: entries.clone().iter().map(|e| e.index).collect()};
            let _ = payload.tx.send(Ok(client_response)).map_err(|err| error!("{} {:?}", CLIENT_RPC_CHAN_ERR, err));
            ApplyLogsTask::Entries{entries, res: None}
        } else {
            ApplyLogsTask::Entries{entries: payload.entries(), res: Some(payload.tx)}
        };

        // Push the task into the pipeline for processing.
        let _ = self.apply_logs_pipeline.unbounded_send(task);
        fut::ok(())
    }
}
