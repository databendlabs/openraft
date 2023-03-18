use std::fmt::Debug;
use std::marker::PhantomData;
use std::ops::RangeBounds;
use std::sync::Arc;

use tokio::time::Instant;

use crate::defensive::check_range_matches_entries;
use crate::engine::LogIdList;
use crate::entry::RaftPayload;
use crate::log_id::RaftLogId;
use crate::utime::UTime;
use crate::EffectiveMembership;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::MembershipState;
use crate::RaftState;
use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::StoredMembership;

/// StorageHelper provides additional methods to access a RaftStorage implementation.
pub struct StorageHelper<'a, C, Sto>
where
    C: RaftTypeConfig,
    Sto: RaftStorage<C>,
{
    pub(crate) sto: &'a mut Sto,
    _p: PhantomData<C>,
}

impl<'a, C, Sto> StorageHelper<'a, C, Sto>
where
    C: RaftTypeConfig,
    Sto: RaftStorage<C>,
{
    pub fn new(sto: &'a mut Sto) -> Self {
        Self {
            sto,
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
    pub async fn get_initial_state(&mut self) -> Result<RaftState<C::NodeId, C::Node>, StorageError<C::NodeId>> {
        let vote = self.sto.read_vote().await?;
        let st = self.sto.get_log_state().await?;
        let mut last_purged_log_id = st.last_purged_log_id;
        let mut last_log_id = st.last_log_id;
        let (last_applied, _) = self.sto.last_applied_state().await?;
        let mem_state = self.get_membership().await?;

        // Clean up dirty state: snapshot is installed but logs are not cleaned.
        if last_log_id < last_applied {
            self.sto.purge_logs_upto(last_applied.unwrap()).await?;
            last_log_id = last_applied;
            last_purged_log_id = last_applied;
        }

        let log_ids = LogIdList::load_log_ids(last_purged_log_id, last_log_id, self).await?;

        let snapshot_meta = self.sto.get_current_snapshot().await?.map(|x| x.meta).unwrap_or_default();

        let now = Instant::now();

        Ok(RaftState {
            committed: last_applied,
            // The initial value for `vote` is the minimal possible value.
            // See: [Conditions for initialization](https://datafuselabs.github.io/openraft/cluster-formation.html#conditions-for-initialization)
            vote: UTime::new(now, vote.unwrap_or_default()),
            purged_next: last_purged_log_id.next_index(),
            log_ids,
            membership_state: mem_state,
            snapshot_meta,

            // -- volatile fields: they are not persisted.
            server_state: Default::default(),
            purge_upto: last_purged_log_id,
        })
    }

    /// Get the log id of the entry at `index`.
    pub async fn get_log_id(&mut self, log_index: u64) -> Result<LogId<C::NodeId>, StorageError<C::NodeId>> {
        let st = self.sto.get_log_state().await?;

        if Some(log_index) == st.last_purged_log_id.index() {
            return Ok(st.last_purged_log_id.unwrap());
        }

        let entries = self.get_log_entries(log_index..=log_index).await?;

        Ok(*entries[0].get_log_id())
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
        let (_, sm_mem) = self.sto.last_applied_state().await?;

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
    #[tracing::instrument(level = "trace", skip(self))]
    pub async fn last_membership_in_log(
        &mut self,
        since_index: u64,
    ) -> Result<Vec<StoredMembership<C::NodeId, C::Node>>, StorageError<C::NodeId>> {
        let st = self.sto.get_log_state().await?;

        let mut end = st.last_log_id.next_index();
        let start = std::cmp::max(st.last_purged_log_id.next_index(), since_index);
        let step = 64;

        let mut res = vec![];

        while start < end {
            let step_start = std::cmp::max(start, end.saturating_sub(step));
            let entries = self.sto.try_get_log_entries(step_start..end).await?;

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

    /// Try to get an log entry.
    ///
    /// It does not return an error if the log entry at `log_index` is not found.
    pub async fn try_get_log_entry(&mut self, log_index: u64) -> Result<Option<C::Entry>, StorageError<C::NodeId>> {
        let mut res = self.sto.try_get_log_entries(log_index..(log_index + 1)).await?;
        Ok(res.pop())
    }

    /// Get a series of log entries from storage.
    ///
    /// Similar to `try_get_log_entries` except an error will be returned if there is an entry not
    /// found in the specified range.
    pub async fn get_log_entries<RB: RangeBounds<u64> + Clone + Debug + Send + Sync>(
        &mut self,
        range: RB,
    ) -> Result<Vec<C::Entry>, StorageError<C::NodeId>> {
        let res = self.sto.try_get_log_entries(range.clone()).await?;

        check_range_matches_entries::<C, _>(range, &res)?;

        Ok(res)
    }
}
