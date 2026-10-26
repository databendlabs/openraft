use openraft::type_config::alias::LogIdOf;
use openraft_memstore::TypeConfig;

/// Acknowledged progress of a sequential client workload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteSummary {
    pub count: u64,
    pub last_log_id: Option<LogIdOf<TypeConfig>>,
}
