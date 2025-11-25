#[cfg(test)]
mod tests;

use std::error::Error;
use std::fmt::Display;
use std::fmt::Formatter;

use validit::Validate;

use crate::RaftTypeConfig;
use crate::log_id_range::LogIdRange;
use crate::progress::replication_id::ReplicationId;
use crate::type_config::alias::LogIdOf;

/// The inflight data being transmitting from leader to a follower/learner.
///
/// If inflight data is non-None, it's waiting for responses from a follower/learner.
/// The follower/learner responds with either `ack()` or `conflict()` to update the state of
/// inflight data.
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
pub(crate) enum Inflight<C>
where C: RaftTypeConfig
{
    None,

    /// Being replicating a series of logs.
    Logs {
        log_id_range: LogIdRange<C>,
        replication_id: ReplicationId,
    },

    /// Being replicating a snapshot.
    Snapshot {
        replication_id: ReplicationId,
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
            Inflight::Logs {
                log_id_range: r,
                replication_id,
            } => write!(f, "Logs:{}, replication_id:{}", r, replication_id),
            Inflight::Snapshot { replication_id } => write!(f, "Snapshot, replication_id:{}", replication_id),
        }
    }
}

impl<C> Inflight<C>
where C: RaftTypeConfig
{
    /// Create inflight state for sending logs.
    pub(crate) fn logs(prev: Option<LogIdOf<C>>, last: Option<LogIdOf<C>>, replication_id: ReplicationId) -> Self {
        #![allow(clippy::nonminimal_bool)]
        if !(prev < last) {
            Self::None
        } else {
            Self::Logs {
                log_id_range: LogIdRange::new(prev, last),
                replication_id,
            }
        }
    }

    /// Create inflight state for sending snapshot.
    pub(crate) fn snapshot(replication_id: ReplicationId) -> Self {
        Self::Snapshot { replication_id }
    }

    #[allow(dead_code)]
    pub(crate) fn with_id(self, id: u64) -> Self {
        match self {
            Inflight::None => Inflight::None,
            Inflight::Logs {
                log_id_range,
                replication_id: _,
            } => Inflight::Logs {
                log_id_range,
                replication_id: ReplicationId::new(id),
            },
            Inflight::Snapshot { replication_id: _ } => Inflight::Snapshot {
                replication_id: ReplicationId::new(id),
            },
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

    // TODO: test match the replication id
    /// Update inflight state when log up to `upto` is acknowledged by a follower/learner.
    pub(crate) fn ack(&mut self, upto: Option<LogIdOf<C>>, from_replication_id: Option<ReplicationId>) {
        match self {
            Inflight::None => {
                unreachable!("no inflight data")
            }
            Inflight::Logs {
                log_id_range,
                replication_id,
            } => {
                if from_replication_id.is_some_and(|from| *replication_id != from) {
                    return;
                }

                *self = {
                    debug_assert!(upto >= log_id_range.prev);
                    debug_assert!(upto <= log_id_range.last);
                    Inflight::logs(upto, log_id_range.last.clone(), *replication_id)
                }
            }
            Inflight::Snapshot { replication_id } => {
                if from_replication_id.is_some_and(|from| *replication_id != from) {
                    return;
                }

                *self = Inflight::None;
            }
        }
    }

    // TODO: test match the replication id
    /// Update inflight state when a conflicting log id is responded by a follower/learner.
    pub(crate) fn conflict(&mut self, _conflict: u64, from_replication_id: Option<ReplicationId>) {
        match self {
            Inflight::None => {
                unreachable!("no inflight data")
            }
            Inflight::Logs {
                log_id_range: _,
                replication_id,
            } => {
                if from_replication_id.is_some_and(|from| *replication_id != from) {
                    return;
                }

                *self = Inflight::None
            }
            Inflight::Snapshot { replication_id } => {
                if from_replication_id.is_some_and(|from| *replication_id != from) {
                    return;
                }

                unreachable!("sending snapshot should not conflict");
            }
        }
    }
}
