use std::fmt;

use display_more::DisplayOptionExt;

use crate::RaftTypeConfig;
use crate::log_id_range::LogIdRange;
use crate::type_config::alias::LogIdOf;

/// The payload specifying which logs to replicate.
#[derive(PartialEq, Eq, Clone, Debug)]
pub(crate) enum Payload<C>
where C: RaftTypeConfig
{
    /// Probe the matching point with logs from the candidate range `(prev, last]`.
    ///
    /// A non-empty range sends one entry-carrying request. An empty range sends one entry-less
    /// request to deliver an updated commit index.
    Probe { log_id_range: LogIdRange<C> },

    /// Replicate logs after `prev` with no upper bound.
    ///
    /// Used for streaming replication where the leader continuously sends new logs.
    /// The `prev` is updated as logs are acknowledged.
    LogsSince { prev: Option<LogIdOf<C>> },
}

impl<C> fmt::Display for Payload<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self {
            Payload::Probe { log_id_range } => {
                write!(f, "Probe{{{}}}", log_id_range)
            }
            Payload::LogsSince { prev } => {
                write!(f, "LogsSince{{{}}}", prev.display(),)
            }
        }
    }
}

impl<C> Payload<C>
where C: RaftTypeConfig
{
    /// Return whether an acknowledgement completes this payload.
    pub(crate) fn is_complete(&self, acked: Option<LogIdOf<C>>) -> bool {
        match self {
            Payload::Probe { log_id_range } => acked >= log_id_range.prev,
            Payload::LogsSince { .. } => false,
        }
    }

    /// Advance after generating a request and return the remaining payload.
    ///
    /// A probe completes after generating its only request. An open-ended stream advances to the
    /// last sent log id.
    #[must_use]
    pub(crate) fn update_sent(self, sent: Option<LogIdOf<C>>) -> Option<Self> {
        match self {
            Payload::Probe { .. } => None,
            Payload::LogsSince { .. } => Some(Payload::LogsSince { prev: sent }),
        }
    }

    /// Apply an acknowledgement and return the remaining payload.
    ///
    /// A probe keeps its candidate range unchanged for retries and completes once a response
    /// confirms `prev`. An open-ended stream never completes here.
    #[must_use]
    pub(crate) fn update_matching(self, matching: Option<LogIdOf<C>>) -> Option<Self> {
        match self {
            Payload::Probe { log_id_range } => {
                if matching >= log_id_range.prev {
                    return None;
                }

                Some(Payload::Probe { log_id_range })
            }
            Payload::LogsSince { .. } => Some(Payload::LogsSince { prev: matching }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Payload;
    use crate::engine::testing::UTConfig;
    use crate::engine::testing::log_id;
    use crate::log_id_range::LogIdRange;

    #[test]
    fn probe_completes_after_any_request() {
        let range = LogIdRange::<UTConfig>::new(Some(log_id(1, 1, 52)), Some(log_id(1, 1, 60)));
        let payload = Payload::Probe { log_id_range: range };

        let updated = payload.update_sent(range.prev);
        assert_eq!(None, updated);

        let payload = Payload::Probe { log_id_range: range };
        let updated = payload.update_sent(Some(log_id(1, 1, 53)));
        assert_eq!(None, updated);
    }

    #[test]
    fn probe_completes_when_prev_is_acknowledged() {
        let range = LogIdRange::<UTConfig>::new(Some(log_id(1, 1, 52)), Some(log_id(1, 1, 60)));
        let payload = Payload::Probe { log_id_range: range };

        let updated = payload.update_matching(Some(log_id(1, 1, 51)));
        assert_eq!(Some(Payload::Probe { log_id_range: range }), updated);

        let payload = updated.unwrap();
        let updated = payload.update_matching(range.prev);
        assert_eq!(None, updated);
    }

    #[test]
    fn fresh_learner_probe_completes_on_empty_acknowledgement() {
        let range = LogIdRange::<UTConfig>::new(None, Some(log_id(1, 1, 8)));
        let payload = Payload::Probe { log_id_range: range };

        let updated = payload.update_matching(None);
        assert_eq!(None, updated);
    }

    #[test]
    fn empty_probe_sends_one_commit_only_request() {
        let matching = Some(log_id(1, 1, 52));
        let range = LogIdRange::<UTConfig>::new(matching, matching);
        let payload = Payload::Probe { log_id_range: range };

        let updated = payload.update_sent(matching);
        assert_eq!(None, updated);

        let payload = Payload::Probe { log_id_range: range };
        let updated = payload.update_matching(matching);
        assert_eq!(None, updated);
    }

    #[test]
    fn pipeline_stays_open() {
        let prev = Some(log_id(1, 1, 52));
        let matching = Some(log_id(1, 1, 60));
        let pipeline = Payload::<UTConfig>::LogsSince { prev };

        let updated = pipeline.update_matching(matching);
        assert_eq!(Some(Payload::LogsSince { prev: matching }), updated);
    }
}
