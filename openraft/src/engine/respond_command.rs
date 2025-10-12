use crate::RaftTypeConfig;
use crate::engine::Respond;

/// A respond waiting for an IO condition to be satisfied.
///
/// Stores the expected progress value that must be reached before sending the respond.
#[derive(Debug)]
pub(crate) struct PendingRespond<C, V>
where C: RaftTypeConfig
{
    /// The expected progress value that must be reached before sending the respond.
    wait_for: V,
    respond: Respond<C>,
}

impl<C, V> PendingRespond<C, V>
where C: RaftTypeConfig
{
    pub(crate) fn new(wait_for: V, respond: Respond<C>) -> Self {
        Self { wait_for, respond }
    }

    pub(crate) fn wait_for(&self) -> &V {
        &self.wait_for
    }

    pub(crate) fn into_respond(self) -> Respond<C> {
        self.respond
    }
}
