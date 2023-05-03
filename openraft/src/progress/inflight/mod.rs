// TODO: remove it
#![allow(unused)]

#[cfg(test)] mod tests;

use std::error::Error;
use std::fmt::Display;
use std::fmt::Formatter;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use crate::less_equal;
use crate::log_id_range::LogIdRange;
use crate::validate::Validate;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::MessageSummary;
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
    Logs {
        id: u64,

        log_id_range: LogIdRange<NID>,
    },

    /// Being replicating a snapshot.
    Snapshot {
        id: u64,

        /// The last log id snapshot includes.
        ///
        /// It is None, if the snapshot is empty.
        last_log_id: Option<LogId<NID>>,
    },
}

impl<NID: NodeId> Validate for Inflight<NID> {
    fn validate(&self) -> Result<(), Box<dyn Error>> {
        match self {
            Inflight::None => Ok(()),
            Inflight::Logs { log_id_range: r, .. } => r.validate(),
            Inflight::Snapshot { .. } => Ok(()),
        }
    }
}

impl<NID: NodeId> Display for Inflight<NID> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Inflight::None => {
                write!(f, "None")
            }
            Inflight::Logs { id, log_id_range: l } => {
                write!(f, "Logs(id={}):{}", id, l)
            }
            Inflight::Snapshot {
                id,
                last_log_id: last_next,
            } => {
                write!(f, "Snapshot(id={}):{}", id, last_next.summary())
            }
        }
    }
}

impl<NID: NodeId> Inflight<NID> {
    pub(crate) fn logs(prev: Option<LogId<NID>>, last: Option<LogId<NID>>) -> Self {
        #![allow(clippy::nonminimal_bool)]
        if !(prev < last) {
            Self::None
        } else {
            Self::Logs {
                id: 0,
                log_id_range: LogIdRange::new(prev, last),
            }
        }
    }

    pub(crate) fn snapshot(snapshot_last_log_id: Option<LogId<NID>>) -> Self {
        Self::Snapshot {
            id: 0,
            last_log_id: snapshot_last_log_id,
        }
    }

    pub(crate) fn with_id(self, id: u64) -> Self {
        match self {
            Inflight::None => Inflight::None,
            Inflight::Logs { id: _, log_id_range } => Inflight::Logs { id, log_id_range },
            Inflight::Snapshot { id: _, last_log_id } => Inflight::Snapshot { id, last_log_id },
        }
    }

    pub(crate) fn is_my_id(&self, res_id: u64) -> bool {
        match self {
            Inflight::None => false,
            Inflight::Logs { id, .. } => *id == res_id,
            Inflight::Snapshot { id, .. } => *id == res_id,
        }
    }

    pub(crate) fn set_id(&mut self, v: u64) {
        match self {
            Inflight::None => {}
            Inflight::Logs { id, .. } => *id = v,
            Inflight::Snapshot { id, .. } => *id = v,
        }
    }

    pub(crate) fn get_id(&self) -> Option<u64> {
        match self {
            Inflight::None => None,
            Inflight::Logs { id, .. } => Some(*id),
            Inflight::Snapshot { id, .. } => Some(*id),
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
    pub(crate) fn ack(&mut self, upto: Option<LogId<NID>>) {
        let request_id = self.get_id();

        match self {
            Inflight::None => {
                unreachable!("no inflight data")
            }
            Inflight::Logs {
                id: _,
                log_id_range: logs,
            } => {
                *self = {
                    debug_assert!(upto >= logs.prev_log_id);
                    debug_assert!(upto <= logs.last_log_id);
                    Inflight::logs(upto, logs.last_log_id)
                }
            }
            Inflight::Snapshot { id: _, last_log_id } => {
                debug_assert_eq!(&upto, last_log_id);
                *self = Inflight::None;
            }
        }

        if let Some(request_id) = request_id {
            self.set_id(request_id);
        }
    }

    /// Update inflight state when a conflicting log id is responded by a follower/learner.
    pub(crate) fn conflict(&mut self, conflict: u64) {
        match self {
            Inflight::None => {
                unreachable!("no inflight data")
            }
            Inflight::Logs {
                id: _,
                log_id_range: logs,
            } => {
                // if prev_log_id==None, it will never conflict
                debug_assert_eq!(Some(conflict), logs.prev_log_id.index());
                *self = Inflight::None
            }
            Inflight::Snapshot { id: _, last_log_id: _ } => {
                unreachable!("sending snapshot should not conflict");
            }
        }
    }
}
