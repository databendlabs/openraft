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

    /// Waiting for an acknowledgement of logs from `(prev, last]`.
    ///
    /// Completes when a response acknowledges `prev` or a later log. Used for matching-point
    /// probes and the leader's local progress update, which acknowledges the entire range
    /// immediately.
    Probe {
        log_id_range: LogIdRange<C>,
        inflight_id: InflightId,
    },

    /// Replicating logs after `prev` with no upper bound (streaming mode).
    ///
    /// Unlike `Probe` which replicates a fixed range, `LogsSince` represents
    /// open-ended streaming replication. The `prev` advances as logs are
    /// acknowledged by the follower.
    LogsSince {
        prev: Option<LogIdOf<C>>,
        inflight_id: InflightId,
    },

    /// Replicating a snapshot.
    Snapshot {
        inflight_id: InflightId,
    },

    /// Waiting for the replication task to confirm that it no longer reads the log for a plan the
    /// leader dropped, a probe or a stream of the logs after `prev`.
    ///
    /// The leader asks with [`Command::ClearReplicate`], and the task answers with the same
    /// `inflight_id` once no read for the dropped plan is running or can start. Until then the
    /// logs after `prev` must not be purged, and no other plan starts.
    ///
    /// [`Command::ClearReplicate`]: crate::engine::Command::ClearReplicate
    Clear {
        prev: Option<LogIdOf<C>>,
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
            Inflight::Probe { log_id_range: r, .. } => r.validate(),
            Inflight::LogsSince { .. } => Ok(()),
            Inflight::Snapshot { .. } => Ok(()),
            Inflight::Clear { .. } => Ok(()),
        }
    }
}

impl<C> Display for Inflight<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Inflight::None => write!(f, "None"),
            Inflight::Probe {
                log_id_range: r,
                inflight_id,
            } => write!(f, "Probe:{}, inflight_id:{}", r, inflight_id),
            Inflight::LogsSince { prev, inflight_id } => {
                write!(f, "LogsSince:{:?}, inflight_id:{}", prev, inflight_id)
            }
            Inflight::Snapshot { inflight_id } => write!(f, "Snapshot, inflight_id:{}", inflight_id),
            Inflight::Clear { prev, inflight_id } => {
                write!(f, "Clear:{:?}, inflight_id:{}", prev, inflight_id)
            }
        }
    }
}

impl<C> Inflight<C>
where C: RaftTypeConfig
{
    /// Create inflight state for probing a matching log id.
    pub(crate) fn probe(prev: Option<LogIdOf<C>>, last: Option<LogIdOf<C>>, inflight_id: InflightId) -> Self {
        #![allow(clippy::nonminimal_bool)]
        if !(prev < last) {
            Self::None
        } else {
            Self::Probe {
                log_id_range: LogIdRange::new(prev, last),
                inflight_id,
            }
        }
    }

    /// Create inflight state for sending snapshot.
    pub(crate) fn snapshot(inflight_id: InflightId) -> Self {
        Self::Snapshot { inflight_id }
    }

    /// Create inflight state for streaming logs after a given log id.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn logs_since(prev: Option<LogIdOf<C>>, inflight_id: InflightId) -> Self {
        Self::LogsSince { prev, inflight_id }
    }

    /// Turn this log plan, which the leader drops, into the [`Inflight::Clear`] that waits for the
    /// replication task to answer `inflight_id`.
    ///
    /// Returns `None` if `self` is not a log plan: no replication task reads the log for it.
    pub(crate) fn into_clear(self, inflight_id: InflightId) -> Option<Self> {
        let prev = match self {
            Inflight::Probe { log_id_range, .. } => log_id_range.prev,
            Inflight::LogsSince { prev, .. } => prev,
            Inflight::None | Inflight::Snapshot { .. } | Inflight::Clear { .. } => return None,
        };

        Some(Self::Clear { prev, inflight_id })
    }

    #[allow(dead_code)]
    pub(crate) fn with_id(self, id: u64) -> Self {
        match self {
            Inflight::None => Inflight::None,
            Inflight::Probe {
                log_id_range,
                inflight_id: _,
            } => Inflight::Probe {
                log_id_range,
                inflight_id: InflightId::new(id),
            },
            Inflight::Snapshot { inflight_id: _ } => Inflight::Snapshot {
                inflight_id: InflightId::new(id),
            },
            Inflight::LogsSince { prev, inflight_id: _ } => Inflight::LogsSince {
                prev,
                inflight_id: InflightId::new(id),
            },
            Inflight::Clear { prev, inflight_id: _ } => Inflight::Clear {
                prev,
                inflight_id: InflightId::new(id),
            },
        }
    }

    pub(crate) fn is_none(&self) -> bool {
        &Inflight::None == self
    }

    // test it if used
    #[allow(dead_code)]
    pub(crate) fn is_probe(&self) -> bool {
        matches!(self, Inflight::Probe { .. })
    }

    pub(crate) fn is_logs_since(&self) -> bool {
        matches!(self, Inflight::LogsSince { .. })
    }

    // test it if used
    #[allow(dead_code)]
    pub(crate) fn is_sending_snapshot(&self) -> bool {
        matches!(self, Inflight::Snapshot { .. })
    }

    /// Return the ID of the operation in flight, or `None` if there is none.
    ///
    /// For [`Inflight::Clear`], it is the ID of the clear request, not that of the dropped plan.
    pub(crate) fn inflight_id(&self) -> Option<InflightId> {
        match self {
            Inflight::None => None,
            Inflight::Probe { inflight_id, .. }
            | Inflight::LogsSince { inflight_id, .. }
            | Inflight::Snapshot { inflight_id }
            | Inflight::Clear { inflight_id, .. } => Some(*inflight_id),
        }
    }

    /// Update inflight state when log up to `upto` is acknowledged by a follower/learner.
    ///
    /// Returns `true` if the ack matched the current inflight request and was applied; `false` if
    /// it was stale and ignored. An ack is stale when its `from_inflight_id` does not match the
    /// current request, or when there is no inflight request at all ([`Inflight::None`]): in
    /// pipeline ([`Inflight::LogsSince`]) mode several acks share one [`InflightId`], and a
    /// concurrent log reversion can clear the inflight to `None` (see
    /// [`Updater::update_conflicting`]) while acks queued before the reset are still arriving. The
    /// caller must not advance `matching` when this returns `false`: the follower state such an ack
    /// reports no longer holds.
    ///
    /// [`Updater::update_conflicting`]: crate::progress::entry::update::Updater::update_conflicting
    pub(crate) fn ack(&mut self, upto: Option<LogIdOf<C>>, from_inflight_id: InflightId) -> bool {
        match self {
            Inflight::None => false,
            Inflight::Probe {
                log_id_range,
                inflight_id,
            } => {
                if *inflight_id != from_inflight_id {
                    return false;
                }

                debug_assert!(upto >= log_id_range.prev);
                debug_assert!(upto <= log_id_range.last);

                if upto >= log_id_range.prev {
                    *self = Inflight::None;
                }
                true
            }
            Inflight::Snapshot { inflight_id } => {
                if *inflight_id != from_inflight_id {
                    return false;
                }

                *self = Inflight::None;
                true
            }
            Inflight::LogsSince { prev, inflight_id } => {
                if *inflight_id != from_inflight_id {
                    return false;
                }

                debug_assert!(&upto >= prev);

                *prev = upto;
                true
            }
            // No follower answers a clear: only the replication task does, see `cleared()`.
            Inflight::Clear { .. } => false,
        }
    }

    /// Update inflight state when a conflicting log id is responded by a follower/learner.
    ///
    /// Returns `true` if the conflict matched the current inflight request and was applied;
    /// `false` if it was stale and ignored.
    pub(crate) fn conflict(&mut self, _conflict: u64, from_inflight_id: InflightId) -> bool {
        match self {
            Inflight::None => false,
            Inflight::Probe {
                log_id_range: _,
                inflight_id,
            } => {
                if *inflight_id != from_inflight_id {
                    return false;
                }

                *self = Inflight::None;
                true
            }
            Inflight::Snapshot { inflight_id } => {
                if *inflight_id != from_inflight_id {
                    return false;
                }

                unreachable!("sending snapshot should not conflict");
            }
            Inflight::LogsSince { prev: _, inflight_id } => {
                if *inflight_id != from_inflight_id {
                    return false;
                }

                *self = Inflight::None;
                true
            }
            Inflight::Clear { .. } => false,
        }
    }

    /// Update inflight state when the replication task confirms that it no longer reads the log for
    /// the plan the leader dropped.
    ///
    /// Returns `true` if the confirmation answered the current [`Inflight::Clear`] and was applied;
    /// `false` if it was stale and ignored.
    pub(crate) fn cleared(&mut self, from_inflight_id: InflightId) -> bool {
        match self {
            Inflight::Clear { inflight_id, .. } if *inflight_id == from_inflight_id => {
                *self = Inflight::None;
                true
            }
            _ => false,
        }
    }
}
