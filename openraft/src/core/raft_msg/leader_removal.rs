use std::fmt;

use display_more::DisplayOptionExt;

use crate::RaftTypeConfig;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::VoteOf;

/// The removal of a Leader from the membership config, as the [`StepDownWatcher`] observed it.
///
/// It names the Leader by its vote, and the membership config that removes it by its log id.
/// When the watcher observed them, that config was the effective one, it was committed, and it did
/// not contain the Leader. A membership change or a new vote that `RaftCore` handles later makes
/// the removal stale, see [`Engine::is_removed_leader()`].
///
/// [`StepDownWatcher`]: crate::core::StepDownWatcher
/// [`Engine::is_removed_leader()`]: crate::engine::Engine::is_removed_leader
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LeaderRemoval<C>
where C: RaftTypeConfig
{
    pub(crate) vote: VoteOf<C>,
    pub(crate) membership_log_id: Option<LogIdOf<C>>,
}

impl<C> fmt::Display for LeaderRemoval<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "LeaderRemoval{{vote: {}, membership_log_id: {}}}",
            self.vote,
            self.membership_log_id.display()
        )
    }
}
