use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use anyerror::AnyError;
use validit::Valid;

use crate::display_ext::DisplayOptionExt;
use crate::engine::LogIdList;
use crate::entry::RaftPayload;
use crate::log_id::RaftLogId;
use crate::raft_state::IOState;
use crate::storage::log_reader_ext::RaftLogReaderExt;
use crate::storage::RaftLogStorage;
use crate::storage::RaftStateMachine;
use crate::type_config::alias::LogIdOf;
use crate::type_config::TypeConfigExt;
use crate::utime::Leased;
use crate::EffectiveMembership;
use crate::LogIdOptionExt;
use crate::MembershipState;
use crate::RaftLogReader;
use crate::RaftSnapshotBuilder;
use crate::RaftState;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::StoredMembership;

/// StorageHelper provides additional methods to access a [`RaftLogStorage`] and
/// [`RaftStateMachine`] implementation.
pub struct StorageHelper<'a, C, LS, SM>
where
    C: RaftTypeConfig,
    LS: RaftLogStorage<C>,
    SM: RaftStateMachine<C>,
{
    pub(crate) log_store: &'a mut LS,
    pub(crate) state_machine: &'a mut SM,
    _p: PhantomData<C>,
}

impl<'a, C, LS, SM> StorageHelper<'a, C, LS, SM>
where
    C: RaftTypeConfig,
    LS: RaftLogStorage<C>,
    SM: RaftStateMachine<C>,
{
    /// Creates a new `StorageHelper` that provides additional functions based on the underlying
    ///  [`RaftLogStorage`] and [`RaftStateMachine`] implementation.
    pub fn new(sto: &'a mut LS, sm: &'a mut SM) -> Self {
        Self {
            log_store: sto,
            state_machine: sm,
            _p: Default::default(),
        }
    }

    // TODO: let RaftStore store node-id.
    //       To achieve this, RaftLogStorage must store node-id
    //       To achieve this, RaftLogStorage has to provide API to initialize with a node id and API to
    //       read node-id
    /// Get Raft's state information from storage.
    ///
    /// When the Raft node is first started, it will call this interface to fetch the last known
    /// state from stable storage.
    pub async fn get_initial_state(&mut self) -> Result<RaftState<C>, StorageError<C>> {
        let mut log_reader = self.log_store.get_log_reader().await;
        let vote = log_reader.read_vote().await?;
        let vote = vote.unwrap_or_default();

        let mut committed = self.log_store.read_committed().await?;

        let st = self.log_store.get_log_state().await?;
        let mut last_purged_log_id = st.last_purged_log_id;
        let mut last_log_id = st.last_log_id;

        let (mut last_applied, _) = self.state_machine.applied_state().await?;

        tracing::info!(
            vote = display(&vote),
            last_purged_log_id = display(last_purged_log_id.display()),
            last_applied = display(last_applied.display()),
            committed = display(committed.display()),
            last_log_id = display(last_log_id.display()),
            "get_initial_state"
        );

        // TODO: It is possible `committed < last_applied` because when installing snapshot,
        //       new committed should be saved, but not yet.
        if committed < last_applied {
            committed = last_applied.clone();
        }

        // Re-apply log entries to recover SM to latest state.
        if last_applied < committed {
            let start = last_applied.next_index();
            let end = committed.next_index();

            self.reapply_committed(start, end).await?;

            last_applied = committed.clone();
        }

        let mem_state = self.get_membership().await?;

        // Clean up dirty state: snapshot is installed but logs are not cleaned.
        if last_log_id < last_applied {
            tracing::info!(
                "Clean the hole between last_log_id({}) and last_applied({}) by purging logs to {}",
                last_log_id.display(),
                last_applied.display(),
                last_applied.display(),
            );

            self.log_store.purge(last_applied.clone().unwrap()).await?;
            last_log_id = last_applied.clone();
            last_purged_log_id = last_applied.clone();
        }

        tracing::info!(
            "load key log ids from ({},{}]",
            last_purged_log_id.display(),
            last_log_id.display()
        );

        let log_id_list = self.get_key_log_ids(last_purged_log_id.clone(), last_log_id.clone()).await?;

        let snapshot = self.state_machine.get_current_snapshot().await?;

        // If there is not a snapshot and there are logs purged, which means the snapshot is not persisted,
        // we just rebuild it so that replication can use it.
        let snapshot = match snapshot {
            None => {
                if last_purged_log_id.is_some() {
                    let mut b = self.state_machine.get_snapshot_builder().await;
                    let s = b.build_snapshot().await?;
                    Some(s)
                } else {
                    None
                }
            }
            s @ Some(_) => s,
        };
        let snapshot_meta = snapshot.map(|x| x.meta).unwrap_or_default();

        let io_state = IOState::new(
            &vote,
            last_applied.clone(),
            snapshot_meta.last_log_id.clone(),
            last_purged_log_id.clone(),
        );

        let now = C::now();

        Ok(RaftState {
            committed: last_applied,
            // The initial value for `vote` is the minimal possible value.
            // See: [Conditions for initialization][precondition]
            //
            // [precondition]: crate::docs::cluster_control::cluster_formation#preconditions-for-initialization
            //
            // TODO: If the lease reverted upon restart,
            //       the lease based linearizable read consistency will be broken.
            //       When lease based read is added, the restarted node must sleep for a while,
            //       before serving.
            vote: Leased::new(now, Duration::default(), vote),
            purged_next: last_purged_log_id.next_index(),
            log_ids: log_id_list,
            membership_state: mem_state,
            snapshot_meta,

            // -- volatile fields: they are not persisted.
            server_state: Default::default(),
            io_state: Valid::new(io_state),
            purge_upto: last_purged_log_id,
        })
    }

    /// Read log entries from [`RaftLogReader`] in chunks, and apply them to the state machine.
    pub(crate) async fn reapply_committed(&mut self, mut start: u64, end: u64) -> Result<(), StorageError<C>> {
        let chunk_size = 64;

        tracing::info!(
            "re-apply log [{}..{}) in {} item chunks to state machine",
            chunk_size,
            start,
            end
        );

        let mut log_reader = self.log_store.get_log_reader().await;

        while start < end {
            let chunk_end = std::cmp::min(end, start + chunk_size);
            let entries = log_reader.try_get_log_entries(start..chunk_end).await?;

            let first = entries.first().map(|x| x.get_log_id().index);
            let last = entries.last().map(|x| x.get_log_id().index);

            let make_err = || {
                let err = AnyError::error(format!(
                    "Failed to get log entries, expected index: [{}, {}), got [{:?}, {:?})",
                    start, chunk_end, first, last
                ));

                tracing::error!("{}", err);
                err
            };

            if first != Some(start) {
                return Err(StorageError::read_log_at_index(start, make_err()));
            }
            if last != Some(chunk_end - 1) {
                return Err(StorageError::read_log_at_index(chunk_end - 1, make_err()));
            }

            tracing::info!(
                "re-apply {} log entries: [{}, {}),",
                chunk_end - start,
                start,
                chunk_end
            );
            self.state_machine.apply(entries).await?;

            start = chunk_end;
        }

        Ok(())
    }

    /// Returns the last 2 membership config found in log or state machine.
    ///
    /// A raft node needs to store at most 2 membership config log:
    /// - The first one must be committed, because raft allows to propose new membership only when
    ///   the previous one is committed.
    /// - The second may be committed or not.
    ///
    /// Because when handling append-entries RPC, (1) a raft follower will delete logs that are
    /// inconsistent with the leader,
    /// and (2) a membership will take effect at once it is written,
    /// a follower needs to revert the effective membership to a previous one.
    ///
    /// And because (3) there is at most one outstanding, uncommitted membership log,
    /// a follower only need to revert at most one membership log.
    ///
    /// Thus a raft node will only need to store at most two recent membership logs.
    pub async fn get_membership(&mut self) -> Result<MembershipState<C>, StorageError<C>> {
        let (last_applied, sm_mem) = self.state_machine.applied_state().await?;

        let log_mem = self.last_membership_in_log(last_applied.next_index()).await?;
        tracing::debug!(membership_in_sm=?sm_mem, membership_in_log=?log_mem, "{}", func_name!());

        // There 2 membership configs in logs.
        if log_mem.len() == 2 {
            return Ok(MembershipState::new(
                Arc::new(EffectiveMembership::new_from_stored_membership(log_mem[0].clone())),
                Arc::new(EffectiveMembership::new_from_stored_membership(log_mem[1].clone())),
            ));
        }

        let effective = if log_mem.is_empty() {
            EffectiveMembership::new_from_stored_membership(sm_mem.clone())
        } else {
            EffectiveMembership::new_from_stored_membership(log_mem[0].clone())
        };

        let res = MembershipState::new(
            Arc::new(EffectiveMembership::new_from_stored_membership(sm_mem)),
            Arc::new(effective),
        );

        Ok(res)
    }

    /// Get the last 2 membership configs found in the log.
    ///
    /// This method returns at most membership logs with greatest log index which is
    /// `>=since_index`. If no such membership log is found, it returns `None`, e.g., when logs
    /// are cleaned after being applied.
    #[tracing::instrument(level = "trace", skip_all)]
    pub async fn last_membership_in_log(
        &mut self,
        since_index: u64,
    ) -> Result<Vec<StoredMembership<C>>, StorageError<C>> {
        let st = self.log_store.get_log_state().await?;

        let mut end = st.last_log_id.next_index();

        tracing::info!("load membership from log: [{}..{})", since_index, end);

        let start = std::cmp::max(st.last_purged_log_id.next_index(), since_index);
        let step = 64;

        let mut res = vec![];
        let mut log_reader = self.log_store.get_log_reader().await;

        while start < end {
            let step_start = std::cmp::max(start, end.saturating_sub(step));
            let entries = log_reader.try_get_log_entries(step_start..end).await?;

            for ent in entries.iter().rev() {
                if let Some(mem) = ent.get_membership() {
                    let em = StoredMembership::new(Some(ent.get_log_id().clone()), mem.clone());
                    res.insert(0, em);
                    if res.len() == 2 {
                        return Ok(res);
                    }
                }
            }

            end = end.saturating_sub(step);
        }

        Ok(res)
    }

    // TODO: store purged: Option<LogId> separately.
    /// Get key-log-ids from the log store.
    ///
    /// Key-log-ids are the first log id of each Leader.
    async fn get_key_log_ids(
        &mut self,
        purged: Option<LogIdOf<C>>,
        last: Option<LogIdOf<C>>,
    ) -> Result<LogIdList<C>, StorageError<C>> {
        let mut log_reader = self.log_store.get_log_reader().await;

        let last = match last {
            None => return Ok(LogIdList::new(vec![])),
            Some(x) => x,
        };

        if purged.index() == Some(last.index) {
            return Ok(LogIdList::new(vec![last]));
        }

        let first = log_reader.get_log_id(purged.next_index()).await?;

        let mut log_ids = log_reader.get_key_log_ids(first..=last).await?;

        if !log_ids.is_empty() {
            if let Some(purged) = purged {
                if purged.leader_id() == log_ids[0].leader_id() {
                    if log_ids.len() >= 2 {
                        log_ids[0] = purged;
                    } else {
                        log_ids.insert(0, purged);
                    }
                } else {
                    log_ids.insert(0, purged);
                }
            }
        }

        Ok(LogIdList::new(log_ids))
    }
}
