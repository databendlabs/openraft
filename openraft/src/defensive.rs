use std::collections::Bound;
use std::fmt::Debug;
use std::ops::RangeBounds;

use async_trait::async_trait;

use crate::types::v065::AppData;
use crate::types::v065::AppDataResponse;
use crate::types::v065::Entry;
use crate::types::v065::HardState;
use crate::types::v065::LogId;
use crate::types::v065::RaftStorage;
use crate::DefensiveError;
use crate::ErrorSubject;
use crate::StorageError;
use crate::Violation;
use crate::Wrapper;

/// Defines methods of defensive checks for RaftStorage.
#[async_trait]
pub trait DefensiveCheck<D, R, T>
where
    D: AppData,
    R: AppDataResponse,
    T: RaftStorage<D, R>,
    Self: Wrapper<T>,
{
    /// Enable or disable defensive check when calling storage APIs.
    fn set_defensive(&self, v: bool);

    fn is_defensive(&self) -> bool;

    /// Ensure that logs that have greater index than last_applied should have greater log_id.
    /// Invariant must hold: `log.log_id.index > last_applied.index` implies `log.log_id > last_applied`.
    async fn defensive_no_dirty_log(&self) -> Result<(), StorageError> {
        if !self.is_defensive() {
            return Ok(());
        }

        let (last_applied, _) = self.inner().last_applied_state().await?;
        let last_log_id = self.inner().last_id_in_log().await?;

        if last_log_id.index > last_applied.index && last_log_id < last_applied {
            return Err(
                DefensiveError::new(ErrorSubject::Log(last_log_id), Violation::DirtyLog {
                    higher_index_log_id: last_log_id,
                    lower_index_log_id: last_applied,
                })
                .into(),
            );
        }

        Ok(())
    }

    /// Ensure that current_term must increment for every update, and for every term there could be only one value for
    /// voted_for.
    async fn defensive_incremental_hard_state(&self, hs: &HardState) -> Result<(), StorageError> {
        if !self.is_defensive() {
            return Ok(());
        }

        let h = self.inner().read_hard_state().await?;

        let curr = h.unwrap_or_default();

        if hs.current_term < curr.current_term {
            return Err(
                DefensiveError::new(ErrorSubject::HardState, Violation::TermNotAscending {
                    curr: curr.current_term,
                    to: hs.current_term,
                })
                .into(),
            );
        }

        if hs.current_term == curr.current_term && curr.voted_for.is_some() && hs.voted_for != curr.voted_for {
            return Err(
                DefensiveError::new(ErrorSubject::HardState, Violation::VotedForChanged {
                    curr,
                    to: hs.clone(),
                })
                .into(),
            );
        }

        Ok(())
    }

    /// The log entries fed into a store must be consecutive otherwise it is a bug.
    async fn defensive_consecutive_input(&self, entries: &[&Entry<D>]) -> Result<(), StorageError> {
        if !self.is_defensive() {
            return Ok(());
        }

        if entries.is_empty() {
            return Ok(());
        }

        let mut prev_log_id = entries[0].log_id;

        for e in entries.iter().skip(1) {
            if e.log_id.index != prev_log_id.index + 1 {
                return Err(DefensiveError::new(ErrorSubject::Logs, Violation::LogsNonConsecutive {
                    prev: prev_log_id,
                    next: e.log_id,
                })
                .into());
            }

            prev_log_id = e.log_id;
        }

        Ok(())
    }

    /// Trying to feed in emtpy entries slice is an inappropriate action.
    ///
    /// The impl has to avoid this otherwise it may be a bug.
    async fn defensive_nonempty_input(&self, entries: &[&Entry<D>]) -> Result<(), StorageError> {
        if !self.is_defensive() {
            return Ok(());
        }

        if entries.is_empty() {
            return Err(DefensiveError::new(ErrorSubject::Logs, Violation::LogsEmpty).into());
        }

        Ok(())
    }

    /// The entries to append has to be last_log_id.index + 1
    async fn defensive_append_log_index_is_last_plus_one(&self, entries: &[&Entry<D>]) -> Result<(), StorageError> {
        if !self.is_defensive() {
            return Ok(());
        }

        let last_id = self.last_log_id().await?;

        let first_id = entries[0].log_id;
        if last_id.index + 1 != first_id.index {
            return Err(
                DefensiveError::new(ErrorSubject::Log(first_id), Violation::LogsNonConsecutive {
                    prev: last_id,
                    next: first_id,
                })
                .into(),
            );
        }

        Ok(())
    }

    /// The entries to append has to be greater than any known log ids
    async fn defensive_append_log_id_gt_last(&self, entries: &[&Entry<D>]) -> Result<(), StorageError> {
        if !self.is_defensive() {
            return Ok(());
        }

        let last_id = self.last_log_id().await?;

        let first_id = entries[0].log_id;
        if first_id < last_id {
            return Err(
                DefensiveError::new(ErrorSubject::Log(first_id), Violation::LogsNonConsecutive {
                    prev: last_id,
                    next: first_id,
                })
                .into(),
            );
        }

        Ok(())
    }

    /// Find the last known log id from log or state machine
    /// If no log id found, the default one `0,0` is returned.
    async fn last_log_id(&self) -> Result<LogId, StorageError> {
        let log_last_id = self.inner().last_id_in_log().await?;
        let (sm_last_id, _) = self.inner().last_applied_state().await?;

        Ok(std::cmp::max(log_last_id, sm_last_id))
    }

    /// The entries to apply to state machien has to be last_applied_log_id.index + 1
    async fn defensive_apply_index_is_last_applied_plus_one(&self, entries: &[&Entry<D>]) -> Result<(), StorageError> {
        if !self.is_defensive() {
            return Ok(());
        }

        let (last_id, _) = self.inner().last_applied_state().await?;

        let first_id = entries[0].log_id;
        if last_id.index + 1 != first_id.index {
            return Err(
                DefensiveError::new(ErrorSubject::Apply(first_id), Violation::ApplyNonConsecutive {
                    prev: last_id,
                    next: first_id,
                })
                .into(),
            );
        }

        Ok(())
    }

    /// The range must not be empty otherwise it is an inappropriate action.
    async fn defensive_nonempty_range<RNG: RangeBounds<u64> + Clone + Debug + Send>(
        &self,
        range: RNG,
    ) -> Result<(), StorageError> {
        if !self.is_defensive() {
            return Ok(());
        }
        let start = match range.start_bound() {
            Bound::Included(i) => Some(*i),
            Bound::Excluded(i) => Some(*i + 1),
            Bound::Unbounded => None,
        };

        let end = match range.end_bound() {
            Bound::Included(i) => Some(*i),
            Bound::Excluded(i) => Some(*i - 1),
            Bound::Unbounded => None,
        };

        if start.is_none() || end.is_none() {
            return Ok(());
        }

        if start > end {
            return Err(DefensiveError::new(ErrorSubject::Logs, Violation::RangeEmpty { start, end }).into());
        }

        Ok(())
    }

    /// Requires a range must be at least half open: (-oo, n] or [n, +oo);
    /// In order to keep logs continuity.
    async fn defensive_half_open_range<RNG: RangeBounds<u64> + Clone + Debug + Send>(
        &self,
        range: RNG,
    ) -> Result<(), StorageError> {
        if !self.is_defensive() {
            return Ok(());
        }

        if let Bound::Unbounded = range.start_bound() {
            return Ok(());
        };

        if let Bound::Unbounded = range.end_bound() {
            return Ok(());
        };

        Err(DefensiveError::new(ErrorSubject::Logs, Violation::RangeNotHalfOpen {
            start: range.start_bound().cloned(),
            end: range.end_bound().cloned(),
        })
        .into())
    }

    /// An range operation such as get or delete has to actually covers some log entries in store.
    async fn defensive_range_hits_logs<RNG: RangeBounds<u64> + Debug + Send>(
        &self,
        range: RNG,
        logs: &[Entry<D>],
    ) -> Result<(), StorageError> {
        if !self.is_defensive() {
            return Ok(());
        }

        {
            let want_first = match range.start_bound() {
                Bound::Included(i) => Some(*i),
                Bound::Excluded(i) => Some(*i + 1),
                Bound::Unbounded => None,
            };

            let first = logs.first().map(|x| x.log_id.index);

            if let Some(want) = want_first {
                if first != want_first {
                    return Err(
                        DefensiveError::new(ErrorSubject::LogIndex(want), Violation::LogIndexNotFound {
                            want,
                            got: first,
                        })
                        .into(),
                    );
                }
            }
        }

        {
            let want_last = match range.end_bound() {
                Bound::Included(i) => Some(*i),
                Bound::Excluded(i) => Some(*i - 1),
                Bound::Unbounded => None,
            };

            let last = logs.last().map(|x| x.log_id.index);

            if let Some(want) = want_last {
                if last != want_last {
                    return Err(
                        DefensiveError::new(ErrorSubject::LogIndex(want), Violation::LogIndexNotFound {
                            want,
                            got: last,
                        })
                        .into(),
                    );
                }
            }
        }

        Ok(())
    }

    /// The log id of the entries to apply has to be greater than the last known one.
    async fn defensive_apply_log_id_gt_last(&self, entries: &[&Entry<D>]) -> Result<(), StorageError> {
        if !self.is_defensive() {
            return Ok(());
        }

        let (last_id, _) = self.inner().last_applied_state().await?;

        let first_id = entries[0].log_id;
        if first_id < last_id {
            return Err(
                DefensiveError::new(ErrorSubject::Apply(first_id), Violation::ApplyNonConsecutive {
                    prev: last_id,
                    next: first_id,
                })
                .into(),
            );
        }

        Ok(())
    }
}
