use crate::RaftTypeConfig;
use crate::entry::RaftEntry;
use crate::entry::RaftPayload;
use crate::raft::responder::core_responder::CoreResponder;
use crate::storage::v2::apply_responder::ApplyResponder;
use crate::storage::v2::apply_responder_inner::ApplyResponderInner;
use crate::type_config::alias::EntryOf;

pub type EntryResponder<C> = (EntryOf<C>, Option<ApplyResponder<C>>);

/// Internal builder for constructing [`EntryResponder`] tuples.
///
/// This struct optimizes memory usage by storing the entry and responder separately,
/// avoiding duplication of log_id and membership data (which are already in the entry).
/// The [`ApplyResponder`] is constructed lazily when [`into_parts()`](Self::into_parts)
/// is called.
///
/// This is an internal implementation detail. User code works with the public
/// [`EntryResponder`] type alias directly.
pub(crate) struct EntryResponderBuilder<C: RaftTypeConfig> {
    pub(crate) entry: C::Entry,
    pub(crate) responder: Option<CoreResponder<C>>,
}

impl<C: RaftTypeConfig> EntryResponderBuilder<C> {
    /// Consume this item and return the entry and optional responder.
    ///
    /// Returns `None` for the responder when this entry has no client waiting for a response
    /// (e.g., entries being applied on followers).
    ///
    /// This method extracts the log_id and membership from the entry to construct
    /// the appropriate [`ApplyResponder`] wrapper when a responder is present.
    pub(crate) fn into_parts(self) -> (C::Entry, Option<ApplyResponder<C>>) {
        let responder = match self.responder {
            None => return (self.entry, None),
            Some(r) => r,
        };

        let log_id = self.entry.log_id();
        let membership = self.entry.get_membership();

        let inner = match membership {
            Some(membership) => ApplyResponderInner::Membership {
                log_id,
                membership,
                responder,
            },
            None => ApplyResponderInner::Normal { log_id, responder },
        };

        (self.entry, Some(ApplyResponder { inner }))
    }
}

impl<C: RaftTypeConfig> std::fmt::Display for EntryResponderBuilder<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "EntryResponderBuilder(log_id={}, has_responder={}, has_membership={})",
            self.entry.log_id(),
            self.responder.is_some(),
            self.entry.get_membership().is_some()
        )
    }
}
