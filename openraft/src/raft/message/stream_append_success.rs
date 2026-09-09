use std::fmt;

use display_more::DisplayOptionExt;
use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::type_config::alias::LogIdOf;

/// Successful outcome of a stream append request.
#[since(version = "0.10.0", change = "distinguish full and partial append success")]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum StreamAppendSuccess<C>
where C: RaftTypeConfig
{
    /// The complete request was accepted; `matching` equals the request's `last_log_id`.
    Full(Option<LogIdOf<C>>),

    /// Only a prefix was accepted; `prev_log_id <= matching < last_log_id`.
    Partial(Option<LogIdOf<C>>),
}

impl<C> fmt::Display for StreamAppendSuccess<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StreamAppendSuccess::Full(log_id) => write!(f, "Full({})", log_id.display()),
            StreamAppendSuccess::Partial(log_id) => write!(f, "Partial({})", log_id.display()),
        }
    }
}
