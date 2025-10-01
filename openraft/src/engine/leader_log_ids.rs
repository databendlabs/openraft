use std::fmt;
use std::ops::RangeInclusive;

use crate::RaftTypeConfig;
use crate::type_config::alias::LogIdOf;

/// The first and the last log id belonging to a Leader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LeaderLogIds<C: RaftTypeConfig> {
    log_id_range: Option<RangeInclusive<LogIdOf<C>>>,
}

impl<C> fmt::Display for LeaderLogIds<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.log_id_range {
            None => write!(f, "None"),
            Some(rng) => write!(f, "({}, {})", rng.start(), rng.end()),
        }
    }
}

impl<C> LeaderLogIds<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(log_id_range: Option<RangeInclusive<LogIdOf<C>>>) -> Self {
        Self { log_id_range }
    }

    /// Used only in tests
    #[allow(dead_code)]
    pub(crate) fn new_single(log_id: LogIdOf<C>) -> Self {
        Self {
            log_id_range: Some(log_id.clone()..=log_id),
        }
    }

    /// Used only in tests
    #[allow(dead_code)]
    pub(crate) fn new_start_end(first: LogIdOf<C>, last: LogIdOf<C>) -> Self {
        Self {
            log_id_range: Some(first..=last),
        }
    }

    pub(crate) fn first(&self) -> Option<&LogIdOf<C>> {
        self.log_id_range.as_ref().map(|x| x.start())
    }

    pub(crate) fn last(&self) -> Option<&LogIdOf<C>> {
        self.log_id_range.as_ref().map(|x| x.end())
    }
}
