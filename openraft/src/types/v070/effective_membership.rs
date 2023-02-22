use serde::Deserialize;
use serde::Serialize;

use super::LogId;
use super::Membership;

/// The currently active membership config.
///
/// It includes:
/// - the id of the log that sets this membership config,
/// - and the config.
///
/// An active config is just the last seen config in raft spec.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveMembership {
    /// The id of the log that applies this membership config
    pub log_id: LogId,

    pub membership: Membership,
}

impl EffectiveMembership {
    pub fn new(log_id: LogId, membership: Membership) -> Self {
        Self { log_id, membership }
    }
}
