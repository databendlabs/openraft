use crate::RaftTypeConfig;
use crate::type_config::alias::LogIdOf;

/// What one AppendEntries stream session observed.
///
/// Unlike `ReplicationProgress::remote_matched`, these acknowledgements do not carry across
/// sessions.
pub(crate) struct SessionOutcome<C>
where C: RaftTypeConfig
{
    /// Whether the response stream was consumed to its end.
    ///
    /// `false` means the caller should back off and open a new stream, leaving the payload
    /// unacknowledged.
    pub(crate) exhausted: bool,

    /// The last matching log id the target acknowledged during this session.
    ///
    /// `None` if no response arrived or the target acknowledged an empty log.
    pub(crate) acked: Option<LogIdOf<C>>,
}

impl<C> SessionOutcome<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(exhausted: bool, acked: Option<LogIdOf<C>>) -> Self {
        Self { exhausted, acked }
    }
}
