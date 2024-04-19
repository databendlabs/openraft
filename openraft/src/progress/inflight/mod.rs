#[cfg(test)] mod tests;

use std::error::Error;
use std::fmt::Display;
use std::fmt::Formatter;

use validit::Validate;

use crate::display_ext::DisplayOptionExt;
use crate::log_id_range::LogIdRange;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::NodeId;

/// The inflight data being transmitting from leader to a follower/learner.
///
/// If inflight data is non-None, it's waiting for responses from a follower/learner.
/// The follower/learner respond with `ack()` or `conflict()` to update the state of inflight data.
#[derive(Clone, Copy, Debug)]
#[derive(PartialEq, Eq)]
pub(crate) enum Inflight<NID: NodeId> {
    None,

    /// Being replicating a series of logs.
    Logs(LogIdRange<NID>),

    /// Being replicating a snapshot.
    ///
    /// The last log id snapshot includes.
    /// It is None, if the snapshot is empty.
    Snapshot(Option<LogId<NID>>),
}

impl<NID: NodeId> Validate for Inflight<NID> {
    fn validate(&self) -> Result<(), Box<dyn Error>> {
        match self {
            Inflight::None => Ok(()),
            Inflight::Logs(rng) => rng.validate(),
            Inflight::Snapshot(_) => Ok(()),
        }
    }
}

impl<NID: NodeId> Display for Inflight<NID> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Inflight::None => write!(f, "None"),
            Inflight::Logs(rng) => write!(f, "Logs:{}", rng),
            Inflight::Snapshot(last_log_id) => {
                write!(f, "Snapshot:{}", last_log_id.display())
            }
        }
    }
}

impl<NID: NodeId> Inflight<NID> {
    /// Create inflight state for sending logs.
    pub(crate) fn logs(prev: Option<LogId<NID>>, last: Option<LogId<NID>>) -> Self {
        #![allow(clippy::nonminimal_bool)]
        if !(prev < last) {
            Self::None
        } else {
            Self::Logs(LogIdRange::new(prev, last))
        }
    }

    /// Create inflight state for sending snapshot.
    pub(crate) fn snapshot(snapshot_last_log_id: Option<LogId<NID>>) -> Self {
        Self::Snapshot(snapshot_last_log_id)
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
    pub(crate) fn ack(&mut self, upto: Option<LogId<NID>>) {
        match self {
            Inflight::None => {
                tracing::debug!("no inflight data, maybe it is a heartbeat or duplicate ack");
            }
            Inflight::Logs(rng) => {
                *self = {
                    debug_assert!(upto >= rng.prev);
                    debug_assert!(upto <= rng.last);
                    Inflight::logs(upto, rng.last)
                }
            }
            Inflight::Snapshot(last_log_id) => {
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
            Inflight::Logs(rng) => {
                // if prev_log_id==None, it will never conflict
                debug_assert_eq!(Some(conflict), rng.prev.index());
                *self = Inflight::None
            }
            Inflight::Snapshot(_last_log_id) => {
                unreachable!("sending snapshot should not conflict");
            }
        };
    }
}
