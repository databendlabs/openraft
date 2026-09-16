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
    /// Replicate logs in a fixed range `(prev, last]`.
    ///
    /// Used for batch replication where the leader sends a known set of log entries.
    LogIdRange { log_id_range: LogIdRange<C> },

    /// Probe the matching point with logs from the candidate range `(prev, last]`.
    ///
    /// Sends one entry-carrying request. See [`LogIdRange::probe_completed_by()`] for the
    /// completion and retry rules shared with the engine.
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
            Payload::LogIdRange { log_id_range } => {
                write!(f, "LogIdRange{{{}}}", log_id_range)
            }
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
    /// Advance the payload and return whether it is complete.
    ///
    /// A probe keeps its candidate range unchanged for retries and completes on the first
    /// acknowledgement beyond `prev`. An open-ended stream never completes here.
    #[must_use]
    pub(crate) fn update_matching(&mut self, matching: Option<LogIdOf<C>>) -> bool {
        match self {
            Payload::LogIdRange { log_id_range } => {
                log_id_range.prev = matching;
                log_id_range.len() == 0
            }
            Payload::Probe { log_id_range } => log_id_range.probe_completed_by(&matching),
            Payload::LogsSince { prev } => {
                *prev = matching;
                false
            }
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
    fn probe_completion_preserves_retry_range() {
        let range = LogIdRange::<UTConfig>::new(Some(log_id(1, 1, 52)), Some(log_id(1, 1, 60)));
        let mut payload = Payload::Probe { log_id_range: range };

        assert!(!payload.update_matching(None));
        assert!(!payload.update_matching(range.prev));
        assert_eq!(Payload::Probe { log_id_range: range }, payload);

        assert!(payload.update_matching(Some(log_id(1, 1, 53))));
    }

    #[test]
    fn fresh_learner_probe_completes_on_first_entry() {
        let range = LogIdRange::<UTConfig>::new(None, Some(log_id(1, 1, 8)));
        let mut payload = Payload::Probe { log_id_range: range };

        assert!(!payload.update_matching(None));
        assert_eq!(Payload::Probe { log_id_range: range }, payload);
        assert!(payload.update_matching(Some(log_id(1, 1, 0))));
    }

    #[test]
    fn fixed_range_completes_but_pipeline_stays_open() {
        let range = LogIdRange::<UTConfig>::new(Some(log_id(1, 1, 52)), Some(log_id(1, 1, 60)));
        let mut payload = Payload::LogIdRange { log_id_range: range };

        assert!(!payload.update_matching(Some(log_id(1, 1, 53))));
        assert!(payload.update_matching(range.last));

        let mut pipeline = Payload::<UTConfig>::LogsSince { prev: range.prev };
        assert!(!pipeline.update_matching(range.last));
        assert_eq!(Payload::LogsSince { prev: range.last }, pipeline);
    }
}
