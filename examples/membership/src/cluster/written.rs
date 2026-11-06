use openraft::RaftTypeConfig;
use openraft::alias::LogIdOf;

/// A write that the leader applied: its key, its value, and its log ID.
pub struct Written<C>
where C: RaftTypeConfig
{
    pub key: &'static str,
    pub value: &'static str,
    pub log_id: LogIdOf<C>,
}
