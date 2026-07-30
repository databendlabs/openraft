//! Shared state holding the snapshot currently being received.

use std::sync::Arc;

use openraft::OptionalSend;
use openraft::RaftTypeConfig;
use openraft::type_config::TypeConfigExt;
use openraft::type_config::alias::MutexOf;

use crate::network_v1::receiver::Streaming;

/// Shared state for receiving snapshot chunks, stored via [`Raft::extension()`].
///
/// This wrapper holds the ongoing snapshot reception state and is stored
/// via [`Raft::extension()`] to track chunk-based snapshot transfers.
///
/// [`Raft::extension()`]: openraft::Raft::extension
pub struct StreamingState<C, SD>
where
    C: RaftTypeConfig,
    SD: OptionalSend + 'static,
{
    pub(crate) streaming: Arc<MutexOf<C, Option<Streaming<C, SD>>>>,
}

impl<C, SD> Clone for StreamingState<C, SD>
where
    C: RaftTypeConfig,
    SD: OptionalSend + 'static,
{
    fn clone(&self) -> Self {
        Self {
            streaming: self.streaming.clone(),
        }
    }
}

impl<C, SD> StreamingState<C, SD>
where
    C: RaftTypeConfig,
    SD: OptionalSend + 'static,
{
    /// Create a new empty streaming state.
    pub fn new() -> Self {
        Self {
            streaming: Arc::new(C::mutex(None)),
        }
    }
}

impl<C, SD> Default for StreamingState<C, SD>
where
    C: RaftTypeConfig,
    SD: OptionalSend + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}
