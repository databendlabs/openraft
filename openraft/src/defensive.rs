use std::collections::Bound;
use std::fmt::Debug;
use std::ops::RangeBounds;

use async_trait::async_trait;

use crate::raft::Entry;
use crate::raft_types::LogIdOptionExt;
use crate::DefensiveError;
use crate::ErrorSubject;
use crate::LogId;
use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::Violation;
use crate::Vote;
use crate::Wrapper;

/// Defines methods of defensive checks for RaftStorage independent of the storage type.
// TODO This can be extended by other methods, as needed. Currently only moved the one for LogReader
pub trait DefensiveCheckBase<C: RaftTypeConfig> {
    /// Enable or disable defensive check when calling storage APIs.
    fn set_defensive(&self, v: bool);

    fn is_defensive(&self) -> bool;

    /// The range must not be empty otherwise it is an inappropriate action.
    fn defensive_nonempty_range<RB: RangeBounds<u64> + Clone + Debug + Send>(
        &self,
        range: RB,
    ) -> Result<(), StorageError<C>> {
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
}

/// Defines methods of defensive checks for RaftStorage.
#[async_trait]
pub trait DefensiveCheck<C, T>: DefensiveCheckBase<C>
where
    C: RaftTypeConfig,
    T: RaftStorage<C>,
    Self: Wrapper<C, T>,
{
    /// Ensure that logs that have greater index than last_applied should have greater log_id.
    /// Invariant must hold: `log.log_id.index > last_applied.index` implies `log.log_id > last_applied`.
    async fn defensive_no_dirty_log(&mut self) -> Result<(), StorageError<C>> {
        if !self.is_defensive() {
            return Ok(());
        }

        let last_log_id = self.inner().get_log_state().await?.last_log_id;
        let (last_applied, _) = self.inner().last_applied_state().await?;

        if last_log_id.index() > last_applied.index() && last_log_id < last_applied {
            return Err(
                DefensiveError::new(ErrorSubject::Log(last_log_id.unwrap()), Violation::DirtyLog {
                    higher_index_log_id: last_log_id.unwrap(),
                    lower_index_log_id: last_applied.unwrap(),
                })
                .into(),
            );
        }

        Ok(())
    }

    /// Ensure that current_term must increment for every update, and for every term there could be only one value for
    /// voted_for.
    async fn defensive_incremental_vote(&mut self, vote: &Vote<C>) -> Result<(), StorageError<C>> {
        if !self.is_defensive() {
            return Ok(());
        }

        let h = self.inner().read_vote().await?;

        let curr = h.unwrap_or_default();

        if vote.term < curr.term {
            return Err(DefensiveError::new(ErrorSubject::Vote, Violation::TermNotAscending {
                curr: curr.term,
                to: vote.term,
            })
            .into());
        }

        if vote >= &curr {
            Ok(())
        } else {
            Err(DefensiveError::new(ErrorSubject::Vote, Violation::NonIncrementalVote { curr, to: *vote }).into())
        }
    }

    /// The log entries fed into a store must be consecutive otherwise it is a bug.
    async fn defensive_consecutive_input(&self, entries: &[&Entry<C>]) -> Result<(), StorageError<C>> {
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
                    prev: Some(prev_log_id),
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
    async fn defensive_nonempty_input(&self, entries: &[&Entry<C>]) -> Result<(), StorageError<C>> {
        if !self.is_defensive() {
            return Ok(());
        }

        if entries.is_empty() {
            return Err(DefensiveError::new(ErrorSubject::Logs, Violation::LogsEmpty).into());
        }

        Ok(())
    }

    /// The entries to append has to be last_log_id.index + 1
    async fn defensive_append_log_index_is_last_plus_one(
        &mut self,
        entries: &[&Entry<C>],
    ) -> Result<(), StorageError<C>> {
        if !self.is_defensive() {
            return Ok(());
        }

        let last_id = self.inner().get_log_state().await?.last_log_id;

        let first_id = entries[0].log_id;
        if last_id.next_index() != first_id.index {
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
    async fn defensive_append_log_id_gt_last(&mut self, entries: &[&Entry<C>]) -> Result<(), StorageError<C>> {
        if !self.is_defensive() {
            return Ok(());
        }

        let last_id = self.inner().get_log_state().await?.last_log_id;

        let first_id = entries[0].log_id;
        // TODO(xp): test first eq last.
        // TODO(xp): test last == None is ok
        if last_id.is_some() && Some(first_id) <= last_id {
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

    async fn defensive_purge_applied_le_last_applied(&mut self, upto: LogId<C>) -> Result<(), StorageError<C>> {
        let (last_applied, _) = self.inner().last_applied_state().await?;
        if Some(upto.index) > last_applied.index() {
            return Err(
                DefensiveError::new(ErrorSubject::Log(upto), Violation::PurgeNonApplied {
                    last_applied,
                    purge_upto: upto,
                })
                .into(),
            );
        }
        Ok(())
    }

    async fn defensive_delete_conflict_gt_last_applied(&mut self, since: LogId<C>) -> Result<(), StorageError<C>> {
        let (last_applied, _) = self.inner().last_applied_state().await?;
        if Some(since.index) <= last_applied.index() {
            return Err(
                DefensiveError::new(ErrorSubject::Log(since), Violation::AppliedWontConflict {
                    last_applied,
                    first_conflict_log_id: since,
                })
                .into(),
            );
        }
        Ok(())
    }

    /// The entries to apply to state machien has to be last_applied_log_id.index + 1
    async fn defensive_apply_index_is_last_applied_plus_one(
        &mut self,
        entries: &[&Entry<C>],
    ) -> Result<(), StorageError<C>> {
        if !self.is_defensive() {
            return Ok(());
        }

        let (last_id, _) = self.inner().last_applied_state().await?;

        let first_id = entries[0].log_id;
        if last_id.next_index() != first_id.index {
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

    /// Requires a range must be at least half open: (-oo, n] or [n, +oo);
    /// In order to keep logs continuity.
    async fn defensive_half_open_range<RB: RangeBounds<u64> + Clone + Debug + Send>(
        &self,
        range: RB,
    ) -> Result<(), StorageError<C>> {
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
    async fn defensive_range_hits_logs<RB: RangeBounds<u64> + Debug + Send>(
        &self,
        range: RB,
        logs: &[Entry<C>],
    ) -> Result<(), StorageError<C>> {
        if !self.is_defensive() {
            return Ok(());
        }

        check_range_matches_entries(range, logs)?;
        Ok(())
    }

    /// The log id of the entries to apply has to be greater than the last known one.
    async fn defensive_apply_log_id_gt_last(&mut self, entries: &[&Entry<C>]) -> Result<(), StorageError<C>> {
        if !self.is_defensive() {
            return Ok(());
        }

        let (last_id, _) = self.inner().last_applied_state().await?;

        let first_id = entries[0].log_id;
        // TODO(xp): test first eq last
        if Some(first_id) <= last_id {
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

pub fn check_range_matches_entries<C: RaftTypeConfig, RB: RangeBounds<u64> + Debug + Send>(
    range: RB,
    entries: &[Entry<C>],
) -> Result<(), StorageError<C>> {
    let want_first = match range.start_bound() {
        Bound::Included(i) => Some(*i),
        Bound::Excluded(i) => Some(*i + 1),
        Bound::Unbounded => None,
    };

    let want_last = match range.end_bound() {
        Bound::Included(i) => Some(*i),
        Bound::Excluded(i) => Some(*i - 1),
        Bound::Unbounded => None,
    };

    if want_first.is_some() && want_last.is_some() && want_first > want_last {
        // empty range
        return Ok(());
    }

    {
        let first = entries.first().map(|x| x.log_id.index);

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
        let last = entries.last().map(|x| x.log_id.index);

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
