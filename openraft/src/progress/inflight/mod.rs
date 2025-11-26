#[cfg(test)]
mod tests;

use std::error::Error;
use std::fmt::Display;
use std::fmt::Formatter;

use validit::Validate;

use crate::RaftTypeConfig;
use crate::log_id_range::LogIdRange;
use crate::progress::inflight_id::InflightId;
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
        inflight_id: InflightId,
    },

    /// Being replicating a snapshot.
    Snapshot {
        inflight_id: InflightId,
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
                inflight_id,
            } => write!(f, "Logs:{}, inflight_id:{}", r, inflight_id),
            Inflight::Snapshot { inflight_id } => write!(f, "Snapshot, inflight_id:{}", inflight_id),
        }
    }
}

impl<C> Inflight<C>
where C: RaftTypeConfig
{
    /// Create inflight state for sending logs.
    pub(crate) fn logs(prev: Option<LogIdOf<C>>, last: Option<LogIdOf<C>>, inflight_id: InflightId) -> Self {
        #![allow(clippy::nonminimal_bool)]
        if !(prev < last) {
            Self::None
        } else {
            Self::Logs {
                log_id_range: LogIdRange::new(prev, last),
                inflight_id,
            }
        }
    }

    /// Create inflight state for sending snapshot.
    pub(crate) fn snapshot(inflight_id: InflightId) -> Self {
        Self::Snapshot { inflight_id }
    }

    #[allow(dead_code)]
    pub(crate) fn with_id(self, id: u64) -> Self {
        match self {
            Inflight::None => Inflight::None,
            Inflight::Logs {
                log_id_range,
                inflight_id: _,
            } => Inflight::Logs {
                log_id_range,
                inflight_id: InflightId::new(id),
            },
            Inflight::Snapshot { inflight_id: _ } => Inflight::Snapshot {
                inflight_id: InflightId::new(id),
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

    /// Update inflight state when log up to `upto` is acknowledged by a follower/learner.
    ///
    /// If `from_inflight_id` doesn't match the current `InflightId`,
    /// the ack is ignored as a stale response.
    pub(crate) fn ack(&mut self, upto: Option<LogIdOf<C>>, from_inflight_id: InflightId) {
        match self {
            Inflight::None => {
                unreachable!("no inflight data")
            }
            Inflight::Logs {
                log_id_range,
                inflight_id,
            } => {
                if *inflight_id != from_inflight_id {
                    return;
                }

                *self = {
                    debug_assert!(upto >= log_id_range.prev);
                    debug_assert!(upto <= log_id_range.last);
                    Inflight::logs(upto, log_id_range.last.clone(), *inflight_id)
                }
            }
            Inflight::Snapshot { inflight_id } => {
                if *inflight_id != from_inflight_id {
                    return;
                }

                *self = Inflight::None;
            }
        }
    }

    /// Update inflight state when a conflicting log id is responded by a follower/learner.
    ///
    /// If `from_inflight_id` doesn't match the current `InflightId`,
    /// the conflict is ignored as a stale response.
    pub(crate) fn conflict(&mut self, _conflict: u64, from_inflight_id: InflightId) {
        match self {
            Inflight::None => {
                unreachable!("no inflight data")
            }
            Inflight::Logs {
                log_id_range: _,
                inflight_id,
            } => {
                if *inflight_id != from_inflight_id {
                    return;
                }

                *self = Inflight::None
            }
            Inflight::Snapshot { inflight_id } => {
                if *inflight_id != from_inflight_id {
                    return;
                }

                unreachable!("sending snapshot should not conflict");
            }
        }
    }
}
