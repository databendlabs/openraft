#[cfg(test)] mod tests;

use std::error::Error;
use std::fmt::Display;
use std::fmt::Formatter;

use validit::Validate;

use crate::log_id_range::LogIdRange;
use crate::LogId;
use crate::LogIdOptionExt;
use crate::MessageSummary;
use crate::NodeId;

#[derive(Debug)]
#[derive(thiserror::Error)]
pub(crate) enum InflightError {
    #[error("got invalid request id, mine={mine:?}, received={received}")]
    InvalidRequestId { mine: Option<u64>, received: u64 },
}

impl InflightError {
    pub(crate) fn invalid_request_id(mine: Option<u64>, received: u64) -> Self {
        Self::InvalidRequestId { mine, received }
    }
}

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
    /// Create inflight state for sending logs.
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

    /// Create inflight state for sending snapshot.
    pub(crate) fn snapshot(snapshot_last_log_id: Option<LogId<NID>>) -> Self {
        Self::Snapshot {
            id: 0,
            last_log_id: snapshot_last_log_id,
        }
    }

    /// Set a request id for identifying response.
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

    pub(crate) fn assert_my_id(&self, res_id: u64) -> Result<(), InflightError> {
        match self {
            Inflight::None => return Err(InflightError::invalid_request_id(None, res_id)),
            Inflight::Logs { id, .. } => {
                if *id != res_id {
                    return Err(InflightError::invalid_request_id(Some(*id), res_id));
                }
            }
            Inflight::Snapshot { id, .. } => {
                if *id != res_id {
                    return Err(InflightError::invalid_request_id(Some(*id), res_id));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn get_id(&self) -> Option<u64> {
        match self {
            Inflight::None => None,
            Inflight::Logs { id, .. } => Some(*id),
            Inflight::Snapshot { id, .. } => Some(*id),
        }
    }

    #[allow(unused)]
    pub(crate) fn id(&self) -> u64 {
        self.get_id().unwrap()
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
    pub(crate) fn ack(&mut self, request_id: u64, upto: Option<LogId<NID>>) -> Result<(), InflightError> {
        let res = self.assert_my_id(request_id);
        if let Err(e) = &res {
            tracing::error!("inflight ack error: {}", e);
            return res;
        }

        match self {
            Inflight::None => {
                unreachable!("no inflight data")
            }
            Inflight::Logs { id, log_id_range } => {
                *self = {
                    debug_assert!(upto >= log_id_range.prev_log_id);
                    debug_assert!(upto <= log_id_range.last_log_id);
                    Inflight::logs(upto, log_id_range.last_log_id).with_id(*id)
                }
            }
            Inflight::Snapshot { id: _, last_log_id } => {
                debug_assert_eq!(&upto, last_log_id);
                *self = Inflight::None;
            }
        }

        Ok(())
    }

    /// Update inflight state when a conflicting log id is responded by a follower/learner.
    pub(crate) fn conflict(&mut self, request_id: u64, conflict: u64) -> Result<(), InflightError> {
        let res = self.assert_my_id(request_id);
        if let Err(e) = &res {
            tracing::error!("inflight ack error: {}", e);
            return res;
        }

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
        };

        Ok(())
    }
}
