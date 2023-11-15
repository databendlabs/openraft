use std::collections::Bound;
use std::fmt::Debug;
use std::ops::RangeBounds;

use crate::log_id::RaftLogId;
use crate::DefensiveError;
use crate::ErrorSubject;
use crate::OptionalSend;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::Violation;

pub fn check_range_matches_entries<C: RaftTypeConfig, RB: RangeBounds<u64> + Debug + OptionalSend>(
    range: RB,
    entries: &[C::Entry],
) -> Result<(), StorageError<C::NodeId>> {
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
        let first = entries.first().map(|x| x.get_log_id().index);

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
        let last = entries.last().map(|x| x.get_log_id().index);

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
