#[cfg(test)]
mod tests;

use std::error::Error;
use std::fmt::Display;
use std::fmt::Formatter;

use validit::Validate;

use crate::display_ext::DisplayOptionExt;
use crate::log_id_range::LogIdRange;
use crate::type_config::alias::LogIdOf;
use crate::LogIdOptionExt;
use crate::RaftTypeConfig;

/// The inflight data being transmitting from leader to a follower/learner.
///
/// If inflight data is non-None, it's waiting for responses from a follower/learner.
/// The follower/learner respond with `ack()` or `conflict()` to update the state of inflight data.
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
pub(crate) enum Inflight<C>
where C: RaftTypeConfig
{
    None,

    /// Being replicating a series of logs.
    Logs {
        log_id_range: LogIdRange<C>,
    },

    /// Being replicating a snapshot.
    Snapshot {
        /// The last log id snapshot includes.
        ///
        /// It is None, if the snapshot is empty.
        last_log_id: Option<LogIdOf<C>>,
    },
}

impl<C> Copy for Inflight<C>
where
    C: RaftTypeConfig,
    LogIdOf<C>: Copy,
{
}

impl<C> Validate for Inflight<C>
where C: RaftTypeConfig
{
    fn validate(&self) -> Result<(), Box<dyn Error>> {
        match self {
            Inflight::None => Ok(()),
            Inflight::Logs { log_id_range: r, .. } => r.validate(),
            Inflight::Snapshot { .. } => Ok(()),
        }
    }
}

impl<C> Display for Inflight<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Inflight::None => write!(f, "None"),
            Inflight::Logs { log_id_range: r } => write!(f, "Logs:{}", r),
            Inflight::Snapshot { last_log_id } => {
                write!(f, "Snapshot:{}", last_log_id.display())
            }
        }
    }
}

impl<C> Inflight<C>
where C: RaftTypeConfig
{
    /// Create inflight state for sending logs.
    pub(crate) fn logs(prev: Option<LogIdOf<C>>, last: Option<LogIdOf<C>>) -> Self {
        #![allow(clippy::nonminimal_bool)]
        if !(prev < last) {
            Self::None
        } else {
            Self::Logs {
                log_id_range: LogIdRange::new(prev, last),
            }
        }
    }

    /// Create inflight state for sending snapshot.
    pub(crate) fn snapshot(snapshot_last_log_id: Option<LogIdOf<C>>) -> Self {
        Self::Snapshot {
            last_log_id: snapshot_last_log_id,
        }
    }

    pub(crate) fn is_none(&self) -> bool {
        &Inflight::None == self
    }

    // test it if used
    #[allow(dead_code)]
    pub(crate) fn is_sending_log(&self) -> bool {
        matches!(self, Inflight::Logs { .. })
    }

    // test it if used
    #[allow(dead_code)]
    pub(crate) fn is_sending_snapshot(&self) -> bool {
        matches!(self, Inflight::Snapshot { .. })
    }

    /// Update inflight state when log upto `upto` is acknowledged by a follower/learner.
    pub(crate) fn ack(&mut self, upto: Option<LogIdOf<C>>) {
        match self {
            Inflight::None => {
                unreachable!("no inflight data")
            }
            Inflight::Logs { log_id_range } => {
                *self = {
                    debug_assert!(upto >= log_id_range.prev);
                    debug_assert!(upto <= log_id_range.last);
                    Inflight::logs(upto, log_id_range.last.clone())
                }
            }
            Inflight::Snapshot { last_log_id } => {
                debug_assert_eq!(&upto, last_log_id);
                *self = Inflight::None;
            }
        }
    }

    /// Update inflight state when a conflicting log id is responded by a follower/learner.
    pub(crate) fn conflict(&mut self, conflict: u64) {
        match self {
            Inflight::None => {
                unreachable!("no inflight data")
            }
            Inflight::Logs { log_id_range: logs } => {
                // if prev_log_id==None, it will never conflict
                debug_assert_eq!(Some(conflict), logs.prev.index());
                *self = Inflight::None
            }
            Inflight::Snapshot { last_log_id: _ } => {
                unreachable!("sending snapshot should not conflict");
            }
        }
    }
}
