use std::marker::PhantomData;
use std::sync::Arc;

use crate::display_ext::DisplayOptionExt;
use crate::engine::LogIdList;
use crate::entry::RaftPayload;
use crate::log_id::RaftLogId;
use crate::raft_state::IOState;
use crate::raft_state::LogIOId;
use crate::storage::RaftLogReaderExt;
use crate::storage::RaftLogStorage;
use crate::storage::RaftStateMachine;
use crate::utime::UTime;
use crate::AsyncRuntime;
use crate::EffectiveMembership;
use crate::Instant;
use crate::LogIdOptionExt;
use crate::MembershipState;
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
    //       To achieve this, RaftStorage must store node-id
    //       To achieve this, RaftStorage has to provide API to initialize with a node id and API to
    // read node-id
    /// Get Raft's state information from storage.
    ///
    /// When the Raft node is first started, it will call this interface to fetch the last known
    /// state from stable storage.
    pub async fn get_initial_state(
        &mut self,
    ) -> Result<RaftState<C::NodeId, C::Node, <C::AsyncRuntime as AsyncRuntime>::Instant>, StorageError<C::NodeId>>
    {
        let vote = self.log_store.read_vote().await?;
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
            committed = last_applied;
        }

        // Re-apply log entries to recover SM to latest state.
        if last_applied < committed {
            let start = last_applied.next_index();
            let end = committed.next_index();

            tracing::info!("re-apply log {}..{} to state machine", start, end);

            let entries = self.log_store.get_log_entries(start..end).await?;
            self.state_machine.apply(entries).await?;

            last_applied = committed;
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

            self.log_store.purge(last_applied.unwrap()).await?;
            last_log_id = last_applied;
            last_purged_log_id = last_applied;
        }

        tracing::info!(
            "load key log ids from ({},{}]",
            last_purged_log_id.display(),
            last_log_id.display()
        );
        let log_ids = LogIdList::load_log_ids(last_purged_log_id, last_log_id, self.log_store).await?;

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

        // TODO: `flushed` is not set.
        let io_state = IOState::new(
            vote,
            LogIOId::default(),
            last_applied,
            snapshot_meta.last_log_id,
            last_purged_log_id,
        );

        let now = <C::AsyncRuntime as AsyncRuntime>::Instant::now();

        Ok(RaftState {
            committed: last_applied,
            // The initial value for `vote` is the minimal possible value.
            // See: [Conditions for initialization](https://datafuselabs.github.io/openraft/cluster-formation.html#conditions-for-initialization)
            vote: UTime::new(now, vote),
            purged_next: last_purged_log_id.next_index(),
            log_ids,
            membership_state: mem_state,
            snapshot_meta,

            // -- volatile fields: they are not persisted.
            server_state: Default::default(),
            accepted: Default::default(),
            io_state,
            snapshot_streaming: None,
            purge_upto: last_purged_log_id,
        })
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
    pub async fn get_membership(&mut self) -> Result<MembershipState<C::NodeId, C::Node>, StorageError<C::NodeId>> {
        let (_, sm_mem) = self.state_machine.applied_state().await?;

        let sm_mem_next_index = sm_mem.log_id().next_index();

        let log_mem = self.last_membership_in_log(sm_mem_next_index).await?;
        tracing::debug!(membership_in_sm=?sm_mem, membership_in_log=?log_mem, "RaftStorage::get_membership");

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
    ) -> Result<Vec<StoredMembership<C::NodeId, C::Node>>, StorageError<C::NodeId>> {
        let st = self.log_store.get_log_state().await?;

        let mut end = st.last_log_id.next_index();
        let start = std::cmp::max(st.last_purged_log_id.next_index(), since_index);
        let step = 64;

        let mut res = vec![];

        while start < end {
            let step_start = std::cmp::max(start, end.saturating_sub(step));
            let entries = self.log_store.try_get_log_entries(step_start..end).await?;

            for ent in entries.iter().rev() {
                if let Some(mem) = ent.get_membership() {
                    let em = StoredMembership::new(Some(*ent.get_log_id()), mem.clone());
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
}
