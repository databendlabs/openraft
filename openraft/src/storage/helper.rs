use std::marker::PhantomData;
use std::sync::Arc;

use crate::engine::LogIdList;
use crate::internal_server_state::InternalServerState;
use crate::EffectiveMembership;
use crate::EntryPayload;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::MembershipState;
use crate::RaftState;
use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::StorageError;

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

    /// Get Raft's state information from storage.
    ///
    /// When the Raft node is first started, it will call this interface to fetch the last known state from stable
    /// storage.
    pub async fn get_initial_state(&mut self) -> Result<RaftState<C::NodeId>, StorageError<C::NodeId>> {
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

        Ok(RaftState {
            last_applied,
            // The initial value for `vote` is the minimal possible value.
            // See: [Conditions for initialization](https://datafuselabs.github.io/openraft/cluster-formation.html#conditions-for-initialization)
            vote: vote.unwrap_or_default(),
            log_ids,
            membership_state: mem_state,

            // -- volatile fields: they are not persisted.
            internal_server_state: InternalServerState::default(),
            committed: None,
            server_state: Default::default(),
        })
    }

    /// Get the log id of the entry at `index`.
    pub async fn get_log_id(&mut self, log_index: u64) -> Result<LogId<C::NodeId>, StorageError<C::NodeId>> {
        let st = self.sto.get_log_state().await?;

        if Some(log_index) == st.last_purged_log_id.index() {
            return Ok(st.last_purged_log_id.unwrap());
        }

        let entries = self.sto.get_log_entries(log_index..=log_index).await?;

        Ok(entries[0].log_id)
    }

    /// Returns the last 2 membership config found in log or state machine.
    ///
    /// A raft node needs to store at most 2 membership config log:
    /// - The first one must be committed, because raft allows to propose new membership only when the previous one is
    ///   committed.
    /// - The second may be committed or not.
    ///
    /// Because when handling append-entries RPC, (1) a raft follower will delete logs that are inconsistent with the
    /// leader,
    /// and (2) a membership will take effect at once it is written,
    /// a follower needs to revert the effective membership to a previous one.
    ///
    /// And because (3) there is at most one outstanding, uncommitted membership log,
    /// a follower only need to revert at most one membership log.
    ///
    /// Thus a raft node will only need to store at most two recent membership logs.
    pub async fn get_membership(&mut self) -> Result<MembershipState<C::NodeId>, StorageError<C::NodeId>> {
        let (_, sm_mem) = self.sto.last_applied_state().await?;

        let sm_mem_next_index = sm_mem.log_id.next_index();

        let log_mem = self.last_membership_in_log(sm_mem_next_index).await?;
        tracing::debug!(membership_in_sm=?sm_mem, membership_in_log=?log_mem, "RaftStorage::get_membership");

        // There 2 membership configs in logs.
        if log_mem.len() == 2 {
            return Ok(MembershipState {
                committed: Arc::new(log_mem[0].clone()),
                effective: Arc::new(log_mem[1].clone()),
            });
        }

        let effective = if log_mem.is_empty() {
            sm_mem.clone()
        } else {
            log_mem[0].clone()
        };

        let res = MembershipState {
            committed: Arc::new(sm_mem),
            effective: Arc::new(effective),
        };

        Ok(res)
    }

    /// Get the last 2 membership configs found in the log.
    ///
    /// This method returns at most membership logs with greatest log index which is `>=since_index`.
    /// If no such membership log is found, it returns `None`, e.g., when logs are cleaned after being applied.
    #[tracing::instrument(level = "trace", skip(self))]
    pub async fn last_membership_in_log(
        &mut self,
        since_index: u64,
    ) -> Result<Vec<EffectiveMembership<C::NodeId>>, StorageError<C::NodeId>> {
        let st = self.sto.get_log_state().await?;

        let mut end = st.last_log_id.next_index();
        let start = std::cmp::max(st.last_purged_log_id.next_index(), since_index);
        let step = 64;

        let mut res = vec![];

        while start < end {
            let step_start = std::cmp::max(start, end.saturating_sub(step));
            let entries = self.sto.try_get_log_entries(step_start..end).await?;

            for ent in entries.iter().rev() {
                if let EntryPayload::Membership(ref mem) = ent.payload {
                    let em = EffectiveMembership::new(Some(ent.log_id), mem.clone());
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
