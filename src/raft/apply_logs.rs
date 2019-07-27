use std::sync::Arc;

use actix::prelude::*;
use futures::sync::oneshot;
use log::{error};

use crate::{
    AppError,
    common::{CLIENT_RPC_CHAN_ERR, ApplyLogsTask, DependencyAddr},
    messages::{ClientPayloadResponse, ClientError, Entry},
    network::RaftNetwork,
    raft::Raft,
    storage::{ApplyToStateMachine, ApplyToStateMachinePayload, GetLogEntries, RaftStorage},
};

impl<E: AppError, N: RaftNetwork<E>, S: RaftStorage<E>> Raft<E, N, S> {
    /// Process tasks for applying logs to the state machine.
    ///
    /// **NOTE WELL:** these operations are strictly pipelined to ensure that these operations
    /// happen in strict order for guaranteed linearizability.
    pub(super) fn process_apply_logs_task(&mut self, ctx: &mut Context<Self>, msg: ApplyLogsTask<E>) -> impl ActorFuture<Actor=Self, Item=(), Error=()> {
        match msg {
            ApplyLogsTask::Entry{entry, chan} => fut::Either::A(self.process_apply_logs_task_with_entries(ctx, entry, chan)),
            ApplyLogsTask::Outstanding => fut::Either::B(self.process_apply_logs_task_outstanding(ctx)),
        }
    }

    /// Apply the given payload of log entries to the state machine.
    fn process_apply_logs_task_with_entries(
        &mut self, _: &mut Context<Self>, entry: Arc<Entry>, chan: Option<oneshot::Sender<Result<ClientPayloadResponse, ClientError<E>>>>,
    ) -> impl ActorFuture<Actor=Self, Item=(), Error=()> {
        // PREVIOUS TERM UNCOMMITTED LOGS CHECK:
        // Here we are checking to see if there are any logs from the previous term which were
        // outstanding, but which are now safe to apply due to being covered by a new definitive
        // commit index from this term.
        let entry_index = entry.index;
        let f = if (self.last_applied + 1) != entry_index {
            fut::Either::A(fut::wrap_future(self.storage.send(GetLogEntries::new(self.last_applied + 1, entry_index)))
                .map_err(|err, act: &mut Self, ctx| {
                    act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftStorage);
                    ClientError::Internal
                })
                .and_then(|res, act, ctx| act.map_fatal_storage_result(ctx, res).map_err(|_, _, _| ClientError::Internal))
                .and_then(|res, act, _| {
                    let line_index = res.iter().last().map(|e| e.index);
                    fut::wrap_future(act.storage.send(ApplyToStateMachine::new(ApplyToStateMachinePayload::Multi(res))))
                        .map_err(|err, act: &mut Self, ctx| {
                            act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftStorage);
                            ClientError::Internal
                        })
                        .and_then(|res, act, ctx| act.map_fatal_storage_result(ctx, res).map_err(|_, _, _| ClientError::Internal))
                        .and_then(move |_, act, _| {
                            // Update state after a success operation on the state machine.
                            if let Some(index) = line_index {
                                act.last_applied = index;
                            }
                            fut::ok(())
                        })
                }))
        } else {
            let res: Result<(), ClientError<E>> = Ok(());
            fut::Either::B(fut::result(res))
        };

        // Resume with the normal flow of logic. Here we are simply taking the payload of entries
        // to be applied to the state machine, applying and then responding as needed.
        let line_index = entry.index;
        f.and_then(move |_, act, _| fut::wrap_future(act.storage.send(ApplyToStateMachine::new(ApplyToStateMachinePayload::Single(entry))))
            .map_err(|err, act: &mut Self, ctx| {
                act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftStorage);
                ClientError::Internal
            })
            .and_then(|res, act, ctx| act.map_fatal_storage_result(ctx, res)
                .map_err(|_, _, _| ClientError::Internal)))

            .then(move |res, act, _| match res {
                Ok(_data) => {
                    // Update state after a success operation on the state machine.
                    act.last_applied = line_index;

                    if let Some(tx) = chan {
                        let _ = tx.send(Ok(ClientPayloadResponse{index: line_index})).map_err(|err| error!("{} {:?}", CLIENT_RPC_CHAN_ERR, err));
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
                let line_index = entries.last().map(|elem| elem.index);
                fut::wrap_future(act.storage.send(ApplyToStateMachine::new(ApplyToStateMachinePayload::Multi(entries))))
                    .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftStorage))
                    .and_then(|res, act, ctx| act.map_fatal_storage_result(ctx, res))
                    .map(move |_, _, _| line_index)
            })

            // Update self to reflect progress on applying logs to the state machine.
            .and_then(move |line_index, act, _| {
                if let Some(index) = line_index {
                    act.last_applied = index;
                }
                fut::ok(())
            }))
    }
}
