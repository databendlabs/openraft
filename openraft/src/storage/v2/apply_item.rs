use crate::RaftTypeConfig;
use crate::entry::RaftEntry;
use crate::entry::RaftPayload;
use crate::raft::responder::core_responder::CoreResponder;
use crate::storage::v2::apply_responder::ApplyResponder;
use crate::storage::v2::apply_responder_inner::ApplyResponderInner;
use crate::type_config::alias::EntryOf;

pub type EntryResponder<C> = (EntryOf<C>, ApplyResponder<C>);

/// An entry to be applied to the state machine, paired with an optional responder.
///
/// This struct optimizes memory usage by storing the entry and responder separately,
/// avoiding duplication of log_id and membership data (which are already in the entry).
/// The [`ApplyResponder`] is constructed lazily when [`into_parts()`](Self::into_parts)
/// is called.
///
/// # Example
///
/// ```ignore
/// use openraft::storage::ApplyItem;
/// use openraft::StorageError;
///
/// async fn apply<I>(&mut self, entries: I) -> Result<(), StorageError<C>>
/// where I: IntoIterator<Item = ApplyItem<C>> {
///     for item in entries {
///         let (entry, responder) = item.into_parts();
///         let response = self.process_entry(&entry)?;
///         responder.send(response);
///     }
///     Ok(())
/// }
/// ```
pub(crate) struct EntryResponderBuilder<C: RaftTypeConfig> {
    pub(crate) entry: C::Entry,
    pub(crate) responder: Option<CoreResponder<C>>,
}

impl<C: RaftTypeConfig> EntryResponderBuilder<C> {
    /// Consume this item and return the entry and responder.
    ///
    /// This method extracts the log_id and membership from the entry to construct
    /// the appropriate [`ApplyResponder`] wrapper.
    pub(crate) fn into_parts(self) -> (C::Entry, ApplyResponder<C>) {
        let log_id = self.entry.log_id();
        let membership = self.entry.get_membership();

        let inner = match self.responder {
            None => ApplyResponderInner::None,
            Some(responder) => match membership {
                Some(membership) => ApplyResponderInner::Membership {
                    log_id,
                    membership,
                    responder,
                },
                None => ApplyResponderInner::Normal { log_id, responder },
            },
        };

        (self.entry, ApplyResponder { inner })
    }
}

impl<C: RaftTypeConfig> std::fmt::Display for EntryResponderBuilder<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ApplyItem(log_id={}, has_responder={}, has_membership={})",
            self.entry.log_id(),
            self.responder.is_some(),
            self.entry.get_membership().is_some()
        )
    }
}
