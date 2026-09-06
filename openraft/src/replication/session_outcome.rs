use crate::RaftTypeConfig;
use crate::type_config::alias::LogIdOf;

/// What one AppendEntries stream session observed.
///
/// Every field describes only the session that produced it. A decision about the payload just sent
/// must not consult [`ReplicationProgress`], which carries acknowledgements across sessions and
/// keeps its value when the target reverts its log.
///
/// [`ReplicationProgress`]: crate::replication::replication_progress::ReplicationProgress
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
    /// `None` when no response arrived, which every caller must treat as an acknowledgement that
    /// advanced nothing: no log id is less than `None`.
    pub(crate) acked: Option<LogIdOf<C>>,
}

impl<C> SessionOutcome<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(exhausted: bool, acked: Option<LogIdOf<C>>) -> Self {
        Self { exhausted, acked }
    }
}
