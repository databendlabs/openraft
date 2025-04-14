use std::collections::BTreeMap;
use std::fmt::Debug;

use anyerror::AnyError;
use openraft_macros::add_async_trait;
use openraft_macros::since;

use super::RaftLogReader;
use crate::alias::AsyncRuntimeOf;
use crate::alias::MpscUnboundedReceiverOf;
use crate::alias::MpscUnboundedSenderOf;
use crate::alias::ResponderOf;
use crate::alias::SnapshotDataOf;
use crate::async_runtime::MpscUnboundedReceiver;
use crate::async_runtime::MpscUnboundedSender;
use crate::base::BoxAny;
use crate::base::BoxAsyncOnceMut;
use crate::core::notification::Notification;
use crate::core::raft_msg::ResultSender;
use crate::core::sm::Command;
use crate::core::sm::CommandResult;
use crate::core::sm::Response;
use crate::core::ApplyResult;
use crate::display_ext::DisplayOptionExt;
use crate::display_ext::DisplaySliceExt;
use crate::entry::RaftEntry;
use crate::entry::RaftPayload;
use crate::error::Infallible;
use crate::raft::responder::Responder;
use crate::raft::ClientWriteResponse;
use crate::raft_state::IOId;
use crate::storage::Snapshot;
use crate::storage::SnapshotMeta;
use crate::type_config::alias::LogIdOf;
use crate::type_config::OneshotSender;
use crate::AsyncRuntime;
use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftSnapshotBuilder;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::StoredMembership;

pub struct BuildSnapshotResultSender<C: RaftTypeConfig> {
    resp_tx: MpscUnboundedSenderOf<C, Notification<C>>,
}

impl<C: RaftTypeConfig> BuildSnapshotResultSender<C> {
    pub(crate) fn new(resp_tx: MpscUnboundedSenderOf<C, Notification<C>>) -> Self {
        Self { resp_tx }
    }

    pub fn send(self, result: Result<Snapshot<C>, StorageError<C>>) {
        let result = result.map(|snap| Response::BuildSnapshot(snap.meta));
        let cmd_res = CommandResult::new(result);
        let _ = self.resp_tx.send(Notification::sm(cmd_res));
    }
}

pub struct InstallSnapshotResultSender<C: RaftTypeConfig> {
    io_id: IOId<C>,
    resp_tx: MpscUnboundedSenderOf<C, Notification<C>>,
}

impl<C: RaftTypeConfig> InstallSnapshotResultSender<C> {
    pub(crate) fn new(io_id: IOId<C>, resp_tx: MpscUnboundedSenderOf<C, Notification<C>>) -> Self {
        Self { io_id, resp_tx }
    }

    pub fn send(self, meta: SnapshotMeta<C>) {
        let res = CommandResult::new(Ok(Response::InstallSnapshot((self.io_id, Some(meta)))));
        let _ = self.resp_tx.send(Notification::sm(res));
    }
}

pub struct ApplyResultSender<C: RaftTypeConfig> {
    since: u64,
    end: u64,
    resp_tx: MpscUnboundedSenderOf<C, Notification<C>>,
}

impl<C: RaftTypeConfig> ApplyResultSender<C> {
    pub(crate) fn new(since: u64, end: u64, resp_tx: MpscUnboundedSenderOf<C, Notification<C>>) -> Self {
        Self { since, end, resp_tx }
    }

    pub fn send(self, result: Result<LogIdOf<C>, StorageError<C>>) {
        let res = CommandResult::new(result.map(|last_applied| {
            Response::Apply(ApplyResult {
                since: self.since,
                end: self.end,
                last_applied,
            })
        }));
        let _ = self.resp_tx.send(Notification::sm(res));
    }
}

/// The payload of a state machine command.
pub enum RaftStateMachineCommand<C>
where C: RaftTypeConfig
{
    /// Instruct the state machine to create a snapshot based on its most recent view.
    BuildSnapshot { tx: BuildSnapshotResultSender<C> },

    /// Get the latest built snapshot.
    GetSnapshot { tx: ResultSender<C, Option<Snapshot<C>>> },

    BeginReceivingSnapshot {
        tx: ResultSender<C, SnapshotDataOf<C>, Infallible>,
    },

    InstallFullSnapshot {
        /// The IO id used to update IO progress.
        ///
        /// Installing a snapshot is considered as an IO of AppendEntries `[0,
        /// snapshot.last_log_id]`
        snapshot: Snapshot<C>,
        tx: InstallSnapshotResultSender<C>,
    },

    /// Apply the log entries to the state machine.
    Apply {
        /// The first log id to apply, inclusive.
        first: LogIdOf<C>,

        /// The last log id to apply, inclusive.
        last: LogIdOf<C>,

        client_resp_channels: BTreeMap<u64, ResponderOf<C>>,

        tx: ApplyResultSender<C>,
    },

    /// Apply a custom function to the state machine.
    ///
    /// To erase the type parameter `SM`, it is a
    /// `Box<dyn FnOnce(&mut SM) -> Box<dyn Future<Output = ()>> + Send + 'static>`
    /// wrapped in a `Box<dyn Any>`
    Func {
        func: BoxAny,
        /// The SM type user specified, for debug purpose.
        input_sm_type: &'static str,
    },
}

impl<C> Debug for RaftStateMachineCommand<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BuildSnapshot { .. } => write!(f, "BuildSnapshot"),
            Self::GetSnapshot { .. } => write!(f, "GetSnapshot"),
            Self::BeginReceivingSnapshot { .. } => write!(f, "BeginReceivingSnapshot"),
            Self::InstallFullSnapshot { snapshot, tx, .. } => {
                write!(
                    f,
                    "InstallFullSnapshot: meta: {:?}, io_id: {:?}",
                    snapshot.meta, tx.io_id
                )
            }
            Self::Apply { first, last, .. } => {
                write!(f, "Apply: [{first},{last}]")
            }
            Self::Func { input_sm_type, .. } => {
                write!(f, "Func({})", input_sm_type)
            }
        }
    }
}

impl<C> RaftStateMachineCommand<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(cmd: Command<C>, resp_tx: &MpscUnboundedSenderOf<C, Notification<C>>) -> Self {
        match cmd {
            Command::BuildSnapshot => Self::BuildSnapshot {
                tx: BuildSnapshotResultSender::new(resp_tx.clone()),
            },
            Command::GetSnapshot { tx } => Self::GetSnapshot { tx },
            Command::BeginReceivingSnapshot { tx } => Self::BeginReceivingSnapshot { tx },
            Command::InstallFullSnapshot { io_id, snapshot } => Self::InstallFullSnapshot {
                snapshot,
                tx: InstallSnapshotResultSender::new(io_id, resp_tx.clone()),
            },
            Command::Apply {
                first,
                last,
                client_resp_channels,
            } => {
                let since = first.index;
                let end = last.index + 1;
                Self::Apply {
                    first,
                    last,
                    client_resp_channels,
                    tx: ApplyResultSender::new(since, end, resp_tx.clone()),
                }
            }
            Command::Func { func, input_sm_type } => Self::Func { func, input_sm_type },
        }
    }
}

/// API for state machine and snapshot.
///
/// Snapshot is part of the state machine, because usually a snapshot is the persisted state of the
/// state machine.
///
/// See: [`StateMachine`](crate::docs::components::state_machine)
#[add_async_trait]
pub trait RaftStateMachine<C>: OptionalSend + OptionalSync + 'static
where C: RaftTypeConfig
{
    /// Snapshot builder type.
    type SnapshotBuilder: RaftSnapshotBuilder<C>;

    /// Run state machine worker on this state machine.
    async fn worker<LR: RaftLogReader<C>>(
        &mut self,
        cmd_rx: &mut MpscUnboundedReceiverOf<C, RaftStateMachineCommand<C>>,
        log_reader: &LR,
    ) -> Result<(), StorageError<C>> {
        loop {
            let cmd = cmd_rx.recv().await;
            let cmd = match cmd {
                None => {
                    tracing::info!("{}: rx closed, state machine worker quit", func_name!());
                    return Ok(());
                }
                Some(x) => x,
            };

            tracing::debug!("{}: received command: {:?}", func_name!(), cmd);

            match cmd {
                RaftStateMachineCommand::BuildSnapshot { tx } => {
                    tracing::info!("{}: build snapshot", func_name!());
                    // TODO: does this need to be abortable?
                    // use futures::future::abortable;
                    // let (fu, abort_handle) = abortable(async move { builder.build_snapshot().await });
                    let mut builder = self.get_snapshot_builder().await;
                    // run the snapshot in a concurrent task
                    let _handle = AsyncRuntimeOf::<C>::spawn(async move {
                        let res = builder.build_snapshot().await;
                        tx.send(res);
                    });
                    tracing::info!("{} returning; spawned snapshot building task", func_name!());
                }
                RaftStateMachineCommand::GetSnapshot { tx } => {
                    tracing::info!("{}: get snapshot", func_name!());
                    let snapshot = self.get_current_snapshot().await?;
                    tracing::info!(
                        "sending back snapshot: meta: {}",
                        snapshot.as_ref().map(|s| &s.meta).display()
                    );
                    let _ = tx.send(Ok(snapshot));
                }
                RaftStateMachineCommand::InstallFullSnapshot { snapshot, tx } => {
                    tracing::info!("{}: install complete snapshot", func_name!());
                    let meta = snapshot.meta.clone();
                    self.install_snapshot(&meta, snapshot.snapshot).await?;
                    tracing::info!("Done install complete snapshot, meta: {}", meta);
                    tx.send(meta);
                }
                RaftStateMachineCommand::BeginReceivingSnapshot { tx } => {
                    tracing::info!("{}: BeginReceivingSnapshot", func_name!());
                    let snapshot_data = self.begin_receiving_snapshot().await?;
                    let _ = tx.send(Ok(snapshot_data));
                }
                RaftStateMachineCommand::Apply {
                    first,
                    last,
                    mut client_resp_channels,
                    tx,
                } => {
                    self.apply_from_log(first, last, log_reader, &mut client_resp_channels, tx).await?;
                }
                RaftStateMachineCommand::Func { func, input_sm_type } => {
                    tracing::debug!("{}: run user defined Func", func_name!());

                    let res: Result<Box<BoxAsyncOnceMut<'static, Self>>, _> = func.downcast();
                    if let Ok(f) = res {
                        f(self).await;
                    } else {
                        tracing::warn!(
                            "User-defined SM function uses incorrect state machine type, expected: {}, got: {}",
                            std::any::type_name::<Self>(),
                            input_sm_type
                        );
                    };
                }
            };
        }
    }

    // TODO: This can be made into sync, provided all state machines will use atomic read or the
    //       like.
    // ---
    /// Returns the last applied log id which is recorded in state machine, and the last applied
    /// membership config.
    ///
    /// ### Correctness requirements
    ///
    /// It is all right to return a membership with greater log id than the
    /// last-applied-log-id.
    /// Because upon startup, the last membership will be loaded by scanning logs from the
    /// `last-applied-log-id`.
    async fn applied_state(&mut self) -> Result<(Option<LogIdOf<C>>, StoredMembership<C>), StorageError<C>>;

    /// Apply the given payload of entries to the state machine.
    ///
    /// The Raft protocol guarantees that only logs which have been _committed_, that is, logs which
    /// have been replicated to a quorum of the cluster, will be applied to the state machine.
    ///
    /// This is where the business logic of interacting with your application's state machine
    /// should live. This is 100% application specific. Perhaps this is where an application
    /// specific transaction is being started, or perhaps committed. This may be where a key/value
    /// is being stored.
    ///
    /// For every entry to apply, an implementation should:
    /// - Store the log id as last applied log id.
    /// - Deal with the business logic log.
    /// - Store membership config if `RaftEntry::get_membership()` returns `Some`.
    ///
    /// Note that for a membership log, the implementation need to do nothing about it, except
    /// storing it.
    ///
    /// An implementation may choose to persist either the state machine or the snapshot:
    ///
    /// - An implementation with persistent state machine: persists the state on disk before
    ///   returning from `apply()`. So that a snapshot does not need to be persistent.
    ///
    /// - An implementation with persistent snapshot: `apply()` does not have to persist state on
    ///   disk. But every snapshot has to be persistent. And when starting up the application, the
    ///   state machine should be rebuilt from the last snapshot.
    async fn apply<I>(&mut self, entries: I) -> Result<Vec<C::R>, StorageError<C>>
    where
        I: IntoIterator<Item = C::Entry> + OptionalSend,
        I::IntoIter: OptionalSend;

    #[tracing::instrument(level = "debug", skip_all)]
    async fn apply_from_log<LR: RaftLogReader<C>>(
        &mut self,
        first: LogIdOf<C>,
        last: LogIdOf<C>,
        log_reader: &LR,
        client_resp_channels: &mut BTreeMap<u64, ResponderOf<C>>,
        final_resp_tx: ApplyResultSender<C>,
    ) -> Result<(), StorageError<C>> {
        // TODO: prepare response before apply,
        //       so that an Entry does not need to be Clone,
        //       and no references will be used by apply

        let since = first.index();
        let end = last.index() + 1;

        let entries = log_reader.try_get_log_entries(since..end).await?;
        if entries.len() != (end - since) as usize {
            return Err(StorageError::read_logs(AnyError::error(format!(
                "returned log entries count({}) does not match the input([{}, {}]))",
                entries.len(),
                since,
                end
            ))));
        }
        tracing::debug!(entries = display(entries.display()), "about to apply");

        let last_applied = last;

        // Fake complain: avoid using `collect()` when not needed
        #[allow(clippy::needless_collect)]
        let applying_entries = entries.iter().map(|e| (e.log_id(), e.get_membership())).collect::<Vec<_>>();

        let n_entries = end - since;

        let apply_results = self.apply(entries).await?;

        let n_replies = apply_results.len() as u64;

        debug_assert_eq!(
            n_entries, n_replies,
            "n_entries: {} should equal n_replies: {}",
            n_entries, n_replies
        );

        let mut results = apply_results.into_iter();
        let mut applying_entries = applying_entries.into_iter();
        for log_index in since..end {
            let (log_id, membership) = applying_entries.next().unwrap();
            let resp = results.next().unwrap();
            let tx = client_resp_channels.remove(&log_index);
            tracing::debug!(
                log_id = debug(&log_id),
                membership = debug(&membership),
                "send_response"
            );

            if let Some(tx) = tx {
                let res = Ok(ClientWriteResponse {
                    log_id,
                    data: resp,
                    membership,
                });
                tx.send(res);
            }
        }

        final_resp_tx.send(Ok(last_applied));
        Ok(())
    }

    /// Get the snapshot builder for the state machine.
    ///
    /// Usually it returns a snapshot view of the state machine(i.e., subsequent changes to the
    /// state machine won't affect the return snapshot view), or just a copy of the entire state
    /// machine.
    ///
    /// The method is intentionally async to give the implementation a chance to use
    /// asynchronous sync primitives to serialize access to the common internal object, if
    /// needed.
    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder;

    /// Create a new blank snapshot, returning a writable handle to the snapshot object.
    ///
    /// Openraft will use this handle to receive snapshot data.
    ///
    /// See the [storage chapter of the guide][sto] for details on log compaction / snapshotting.
    ///
    /// [sto]: crate::docs::getting_started#3-implement-raftlogstorage-and-raftstatemachine
    #[since(version = "0.10.0", change = "SnapshotData without Box")]
    async fn begin_receiving_snapshot(&mut self) -> Result<C::SnapshotData, StorageError<C>>;

    /// Install a snapshot which has finished streaming from the leader.
    ///
    /// Before this method returns:
    /// - The state machine should be replaced with the new contents of the snapshot,
    /// - the input snapshot should be saved, i.e., [`Self::get_current_snapshot`] should return it.
    /// - and all other snapshots should be deleted at this point.
    ///
    /// ### snapshot
    ///
    /// A snapshot created from an earlier call to `begin_receiving_snapshot` which provided the
    /// snapshot.
    #[since(version = "0.10.0", change = "SnapshotData without Box")]
    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<C>,
        snapshot: C::SnapshotData,
    ) -> Result<(), StorageError<C>>;

    /// Get a readable handle to the current snapshot.
    ///
    /// ### implementation algorithm
    ///
    /// Implementing this method should be straightforward. Check the configured snapshot
    /// directory for any snapshot files. A proper implementation will only ever have one
    /// active snapshot, though another may exist while it is being created. As such, it is
    /// recommended to use a file naming pattern which will allow for easily distinguishing between
    /// the current live snapshot, and any new snapshot which is being created.
    ///
    /// A proper snapshot implementation will store last-applied-log-id and the
    /// last-applied-membership config as part of the snapshot, which should be decoded for
    /// creating this method's response data.
    async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot<C>>, StorageError<C>>;
}
