use futures::future::AbortHandle;
use tokio::task::JoinHandle;

/// The Raft node is building snapshot itself.
pub(crate) struct Building {
    /// A handle to abort the building snapshot process early if needed.
    #[allow(dead_code)]
    pub(crate) abort_handle: AbortHandle,

    /// A handle to join the building snapshot process.
    #[allow(dead_code)]
    pub(crate) join_handle: JoinHandle<()>,
}
