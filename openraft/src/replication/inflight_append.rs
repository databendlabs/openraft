use std::fmt;

use crate::RaftTypeConfig;
use crate::display_ext::DisplayOptionExt;
use crate::display_ext::display_instant::DisplayInstantExt;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::LogIdOf;

/// Tracks metadata for a single in-flight AppendEntries request.
///
/// Used by [`InflightAppendQueue`](`super::inflight_append_queue::InflightAppendQueue`)
/// to correlate responses with requests and measure round-trip time.
#[derive(Debug, Clone)]
pub(crate) struct InflightAppend<C>
where C: RaftTypeConfig
{
    /// The time when the AppendEntries request was created.
    pub(crate) sending_time: InstantOf<C>,

    /// The last log id included in this request.
    pub(crate) last_log_id: Option<LogIdOf<C>>,
}

impl<C> fmt::Display for InflightAppend<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "InflightAppend{{sending_time:{}, last_log_id:{}}}",
            self.sending_time.display(),
            self.last_log_id.display()
        )
    }
}

impl<C> InflightAppend<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(last_log_id: Option<LogIdOf<C>>) -> Self {
        Self {
            sending_time: C::now(),
            last_log_id,
        }
    }
}
