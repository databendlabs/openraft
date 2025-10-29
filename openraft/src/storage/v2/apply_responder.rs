use crate::RaftTypeConfig;
use crate::storage::v2::apply_responder_inner::ApplyResponderInner;

/// Responder for sending client write responses after applying an entry.
///
/// This wrapper enables zero-allocation response handling by allowing state machines
/// to send responses immediately after applying each entry, rather than buffering
/// them in a Vec.
///
/// # Construction
///
/// This type cannot be constructed by user code. Instances are provided by
/// Openraft when entries are passed to [`RaftStateMachine::apply`](super::RaftStateMachine::apply).
/// State machine implementations should call [`send()`](Self::send) to
/// return the response after applying each entry.
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
pub struct ApplyResponder<C: RaftTypeConfig> {
    pub(crate) inner: ApplyResponderInner<C>,
}

impl<C: RaftTypeConfig> ApplyResponder<C> {
    pub(crate) fn new_none() -> Self {
        Self {
            inner: ApplyResponderInner::None,
        }
    }

    /// Send the response after applying an entry.
    pub fn send(self, response: C::R) {
        self.inner.send(response)
    }
}
