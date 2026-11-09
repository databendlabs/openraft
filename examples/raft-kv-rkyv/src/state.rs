use std::collections::BTreeMap;

use serde::Deserialize;
use serde::Serialize;

use crate::LogId;
use crate::StoredMembership;
use crate::archive_buffer::ArchiveBuffer;

/// Snapshot contents. Values remain archive handles even in the state machine.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct State {
    pub(crate) last_applied: Option<LogId>,
    pub(crate) membership: StoredMembership,
    pub(crate) values: BTreeMap<String, ArchiveBuffer>,
}
