use std::fmt;

use crate::RaftTypeConfig;
use crate::display_ext::DisplayOptionExt;
use crate::type_config::alias::LogIdOf;

/// The greatest log id confirmed to match the leader's log after a successful AppendEntries.
///
/// This is the last log id that the follower **knows agrees with the leader** — i.e., entries
/// up to and including this id are consistent with the leader's log.  It is not necessarily
/// the last log id present in local storage: the follower may hold additional entries beyond
/// this point that have not yet been validated against the leader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchedLogId<C: RaftTypeConfig> {
    pub log_id: Option<LogIdOf<C>>,
}

impl<C: RaftTypeConfig> MatchedLogId<C> {
    pub fn new(log_id: Option<LogIdOf<C>>) -> Self {
        Self { log_id }
    }
}

impl<C: RaftTypeConfig> fmt::Display for MatchedLogId<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "matched:{}", self.log_id.display())
    }
}
