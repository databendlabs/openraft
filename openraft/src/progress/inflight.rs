// TODO: remove it
#![allow(unused)]

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

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use crate::log_id_range::LogIdRange;
    use crate::progress::Inflight;
    use crate::validate::Validate;
    use crate::LeaderId;
    use crate::LogId;

    fn log_id(index: u64) -> LogId<u64> {
        LogId {
            leader_id: LeaderId { term: 1, node_id: 1 },
            index,
        }
    }

    #[test]
    fn test_inflight_create() -> anyhow::Result<()> {
        // Logs
        let l = Inflight::logs(Some(log_id(5)), Some(log_id(10)));
        assert_eq!(
            Inflight::Logs {
                id: 0,
                log_id_range: LogIdRange::new(Some(log_id(5)), Some(log_id(10)))
            },
            l
        );

        // Empty range
        let l = Inflight::logs(Some(log_id(11)), Some(log_id(10)));
        assert_eq!(Inflight::None, l);
        assert!(l.is_none());

        // Snapshot
        let l = Inflight::snapshot(Some(log_id(10)));
        assert_eq!(
            Inflight::Snapshot {
                id: 0,
                last_log_id: Some(log_id(10))
            },
            l
        );
        assert!(!l.is_none());

        Ok(())
    }

    #[test]
    fn test_inflight_is_xxx() -> anyhow::Result<()> {
        let l = Inflight::<u64>::None;
        assert!(l.is_none());

        let l = Inflight::logs(Some(log_id(5)), Some(log_id(10)));
        assert!(l.is_sending_log());

        let l = Inflight::snapshot(Some(log_id(10)));
        assert!(l.is_sending_snapshot());

        Ok(())
    }

    #[test]
    fn test_inflight_ack_without_inflight_data() -> anyhow::Result<()> {
        let res = std::panic::catch_unwind(|| {
            let mut f = Inflight::<u64>::None;
            f.ack(Some(log_id(4)));
        });
        tracing::info!("res: {:?}", res);
        assert!(res.is_err(), "Inflight::None can not ack");
        Ok(())
    }

    #[test]
    fn test_inflight_ack() -> anyhow::Result<()> {
        // Update matching when transmitting by logs
        {
            let mut f = Inflight::logs(Some(log_id(5)), Some(log_id(10)));

            f.ack(Some(log_id(5)));
            assert_eq!(Inflight::logs(Some(log_id(5)), Some(log_id(10))), f);

            f.ack(Some(log_id(6)));
            assert_eq!(Inflight::logs(Some(log_id(6)), Some(log_id(10))), f);

            f.ack(Some(log_id(9)));
            assert_eq!(Inflight::logs(Some(log_id(9)), Some(log_id(10))), f);

            f.ack(Some(log_id(10)));
            assert_eq!(Inflight::None, f);

            {
                let res = std::panic::catch_unwind(|| {
                    let mut f = Inflight::logs(Some(log_id(5)), Some(log_id(10)));
                    f.ack(Some(log_id(4)));
                });
                tracing::info!("res: {:?}", res);
                assert!(res.is_err(), "non-matching ack < prev_log_id");
            }

            {
                let res = std::panic::catch_unwind(|| {
                    let mut f = Inflight::logs(Some(log_id(5)), Some(log_id(10)));
                    f.ack(Some(log_id(11)));
                });
                tracing::info!("res: {:?}", res);
                assert!(res.is_err(), "non-matching ack > prev_log_id");
            }
        }

        // Update matching when transmitting by snapshot
        {
            {
                let mut f = Inflight::snapshot(Some(log_id(5)));
                f.ack(Some(log_id(5)));
                assert_eq!(Inflight::None, f, "valid ack");
            }

            {
                let res = std::panic::catch_unwind(|| {
                    let mut f = Inflight::snapshot(Some(log_id(5)));
                    f.ack(Some(log_id(4)));
                });
                tracing::info!("res: {:?}", res);
                assert!(res.is_err(), "non-matching ack != snapshot.last_log_id");
            }
        }

        Ok(())
    }

    #[test]
    fn test_inflight_conflict() -> anyhow::Result<()> {
        {
            let mut f = Inflight::logs(Some(log_id(5)), Some(log_id(10)));
            f.conflict(5);
            assert_eq!(Inflight::None, f, "valid conflict");
        }

        {
            let res = std::panic::catch_unwind(|| {
                let mut f = Inflight::logs(Some(log_id(5)), Some(log_id(10)));
                f.conflict(4);
            });
            tracing::info!("res: {:?}", res);
            assert!(res.is_err(), "non-matching conflict < prev_log_id");
        }

        {
            let res = std::panic::catch_unwind(|| {
                let mut f = Inflight::logs(Some(log_id(5)), Some(log_id(10)));
                f.conflict(6);
            });
            tracing::info!("res: {:?}", res);
            assert!(res.is_err(), "non-matching conflict > prev_log_id");
        }

        {
            let res = std::panic::catch_unwind(|| {
                let mut f = Inflight::<u64>::None;
                f.conflict(5);
            });
            tracing::info!("res: {:?}", res);
            assert!(res.is_err(), "conflict is not expected by Inflight::None");
        }

        {
            let res = std::panic::catch_unwind(|| {
                let mut f = Inflight::snapshot(Some(log_id(5)));
                f.conflict(5);
            });
            tracing::info!("res: {:?}", res);
            assert!(res.is_err(), "conflict is not expected by Inflight::Snapshot");
        }

        Ok(())
    }

    #[test]
    fn test_inflight_validate() -> anyhow::Result<()> {
        let r = Inflight::Logs {
            id: 0,
            log_id_range: LogIdRange::new(Some(log_id(5)), Some(log_id(4))),
        };
        let res = r.validate();
        assert!(res.is_err(), "prev(5) > last(4)");

        Ok(())
    }
}
