use std::fmt;

use crate::alias::LogIdOf;
use crate::RaftTypeConfig;

/// The first and the last log id belonging to a Leader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LeaderLogIds<C: RaftTypeConfig> {
    first_last: Option<(LogIdOf<C>, LogIdOf<C>)>,
}

impl<C> fmt::Display for LeaderLogIds<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.first_last {
            None => write!(f, "None"),
            Some((first, last)) => write!(f, "({}, {})", first, last),
        }
    }
}

impl<C> LeaderLogIds<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(log_ids: Option<(LogIdOf<C>, LogIdOf<C>)>) -> Self {
        Self { first_last: log_ids }
    }

    /// Used only in tests
    #[allow(dead_code)]
    pub(crate) fn new1(log_id: LogIdOf<C>) -> Self {
        Self {
            first_last: Some((log_id.clone(), log_id)),
        }
    }

    /// Used only in tests
    #[allow(dead_code)]
    pub(crate) fn new2(first: LogIdOf<C>, last: LogIdOf<C>) -> Self {
        Self {
            first_last: Some((first, last)),
        }
    }

    pub(crate) fn first(&self) -> Option<&LogIdOf<C>> {
        self.first_last.as_ref().map(|x| &x.0)
    }

    pub(crate) fn last(&self) -> Option<&LogIdOf<C>> {
        self.first_last.as_ref().map(|x| &x.1)
    }
}
