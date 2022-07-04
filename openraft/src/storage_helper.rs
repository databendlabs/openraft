use std::fmt::Debug;
use std::marker::PhantomData;
use std::ops::RangeBounds;

use crate::defensive::check_range_matches_entries;
use crate::AppData;
use crate::AppDataResponse;
use crate::EffectiveMembership;
use crate::Entry;
use crate::EntryPayload;
use crate::InitialState;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::RaftStorage;
use crate::StorageError;

/// StorageHelper provides additional methods to access a RaftStorage implementation.
pub struct StorageHelper<'a, D, R, S, Sto>
where
    D: AppData,
    R: AppDataResponse,

    S: RaftStorage<D, R>,
    Sto: AsRef<S>,
{
    pub(crate) sto: &'a Sto,
    _p: PhantomData<(D, R, S)>,
}

impl<'a, D, R, S, Sto> StorageHelper<'a, D, R, S, Sto>
where
    D: AppData,
    R: AppDataResponse,
    S: RaftStorage<D, R>,
    Sto: AsRef<S>,
{
    pub fn new(sto: &'a Sto) -> Self {
        Self {
            sto,
            _p: Default::default(),
        }
    }

    /// Get Raft's state information from storage.
    ///
    /// When the Raft node is first started, it will call this interface to fetch the last known state from stable
    /// storage.
    pub async fn get_initial_state(&self) -> Result<InitialState, StorageError> {
        let hs = self.sto.as_ref().read_hard_state().await?;
        let st = self.sto.as_ref().get_log_state().await?;
        let mut last_log_id = st.last_log_id;
        let (last_applied, _) = self.sto.as_ref().last_applied_state().await?;
        let membership = self.get_membership().await?;

        // Clean up dirty state: snapshot is installed but logs are not cleaned.
        if last_log_id < last_applied {
            self.sto.as_ref().purge_logs_upto(last_applied.unwrap()).await?;
            last_log_id = last_applied;
        }

        Ok(InitialState {
            last_log_id,
            last_applied,
            hard_state: hs.unwrap_or_default(),
            last_membership: membership,
        })
    }

    /// Returns the last membership config found in log or state machine.
    pub async fn get_membership(&self) -> Result<Option<EffectiveMembership>, StorageError> {
        let (_, sm_mem) = self.sto.as_ref().last_applied_state().await?;

        let sm_mem_index = match &sm_mem {
            None => 0,
            Some(mem) => mem.log_id.index,
        };

        let log_mem = self.last_membership_in_log(sm_mem_index + 1).await?;

        if log_mem.is_some() {
            return Ok(log_mem);
        }

        Ok(sm_mem)
    }

    /// Get the latest membership config found in the log.
    ///
    /// This method should returns membership with the greatest log index which is `>=since_index`.
    /// If no such membership log is found, it returns `None`, e.g., when logs are cleaned after being applied.
    #[tracing::instrument(level = "trace", skip(self))]
    pub async fn last_membership_in_log(&self, since_index: u64) -> Result<Option<EffectiveMembership>, StorageError> {
        let st = self.sto.as_ref().get_log_state().await?;

        let mut end = st.last_log_id.next_index();
        let start = std::cmp::max(st.last_purged_log_id.next_index(), since_index);
        let step = 64;

        while start < end {
            let step_start = std::cmp::max(start, end.saturating_sub(step));
            let entries = self.sto.as_ref().try_get_log_entries(step_start..end).await?;

            for ent in entries.iter().rev() {
                if let EntryPayload::Membership(ref mem) = ent.payload {
                    return Ok(Some(EffectiveMembership {
                        log_id: ent.log_id,
                        membership: mem.clone(),
                    }));
                }
            }

            end = end.saturating_sub(step);
        }

        Ok(None)
    }

    /// Get a series of log entries from storage.
    ///
    /// Similar to `try_get_log_entries` except an error will be returned if there is an entry not found in the
    /// specified range.
    pub async fn get_log_entries<RB: RangeBounds<u64> + Clone + Debug + Send + Sync>(
        &self,
        range: RB,
    ) -> Result<Vec<Entry<D>>, StorageError> {
        let res = self.sto.as_ref().try_get_log_entries(range.clone()).await?;

        check_range_matches_entries(range, &res)?;

        Ok(res)
    }

    /// Try to get an log entry.
    ///
    /// It does not return an error if the log entry at `log_index` is not found.
    pub async fn try_get_log_entry(&self, log_index: u64) -> Result<Option<Entry<D>>, StorageError> {
        let mut res = self.sto.as_ref().try_get_log_entries(log_index..(log_index + 1)).await?;
        Ok(res.pop())
    }

    /// Get the log id of the entry at `index`.
    pub async fn get_log_id(&self, log_index: u64) -> Result<LogId, StorageError> {
        let st = self.sto.as_ref().get_log_state().await?;

        if Some(log_index) == st.last_purged_log_id.index() {
            return Ok(st.last_purged_log_id.unwrap());
        }

        let entries = self.get_log_entries(log_index..=log_index).await?;

        Ok(entries[0].log_id)
    }
}
