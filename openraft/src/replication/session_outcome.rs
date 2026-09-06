use crate::RaftTypeConfig;
use crate::replication::payload::Payload;
use crate::type_config::alias::LogIdOf;

/// What one AppendEntries stream session observed.
pub(crate) struct SessionOutcome<C>
where C: RaftTypeConfig
{
    /// The greatest matching log id the target acknowledged during this session.
    ///
    /// The outer `None` means no response arrived. `Some(None)` means a response acknowledged an
    /// empty log.
    pub(crate) acked: Option<Option<LogIdOf<C>>>,

    /// The payload left after applying this session's acknowledgement.
    ///
    /// `None` also tells the caller to wait for RaftCore when the session was interrupted.
    pub(crate) remaining: Option<Payload<C>>,
}

impl<C> SessionOutcome<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(acked: Option<Option<LogIdOf<C>>>, remaining: Option<Payload<C>>) -> Self {
        Self { acked, remaining }
    }
}
