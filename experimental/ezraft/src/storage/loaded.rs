//! What storage hands back at startup

use crate::meta::EzMeta;
use crate::snapshot::EzSnapshot;

/// What [`crate::EzStorage::load`] hands back to the framework at startup
///
/// The read-side counterpart of [`crate::Persist`]: `Persist` describes what gets written,
/// `Loaded` is what must come back on the next start.
#[derive(Debug)]
pub struct Loaded {
    /// Raft metadata as last persisted, or [`EzMeta::default`] on first run
    pub meta: EzMeta,

    /// The last persisted snapshot, if any
    pub snapshot: Option<EzSnapshot>,
}
