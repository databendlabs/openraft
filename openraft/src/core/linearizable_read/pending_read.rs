use crate::RaftTypeConfig;
use crate::core::raft_msg::ClientReadTx;
use crate::raft::linearizable_read::Linearizer;
use crate::type_config::alias::InstantOf;

/// A linearizable read waiting for the quorum-acknowledgement clock to satisfy its threshold.
pub(crate) struct PendingRead<C>
where C: RaftTypeConfig
{
    /// The deadline for responding to this read request.
    pub(super) deadline: InstantOf<C>,

    /// The Linearizer to send back to the client.
    pub(super) linearizer: Linearizer<C>,

    /// The channel to send the linearizer back to the client.
    pub(super) response_tx: ClientReadTx<C>,
}

impl<C> PendingRead<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(deadline: InstantOf<C>, linearizer: Linearizer<C>, response_tx: ClientReadTx<C>) -> Self {
        Self {
            deadline,
            linearizer,
            response_tx,
        }
    }
}
