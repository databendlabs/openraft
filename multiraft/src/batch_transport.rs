use std::future::Future;

use openraft::OptionalSend;
use openraft::OptionalSync;
use openraft::RaftTypeConfig;
use openraft::base::BoxStream;
use openraft::error::RPCError;
use openraft::network::RPCOption;
use openraft::openraft_macros::since;

use crate::BatchItem;
use crate::BatchReply;

/// Sends a batch of AppendEntries requests to one peer node in one exchange.
///
/// [`BatchingRouter`](crate::BatchingRouter) calls it with the empty AppendEntries requests that
/// several Raft groups send to the same node. The application owns the connection, the wire format
/// and the receiving side, which can run each item against its group with
/// [`dispatch_batch()`](crate::dispatch_batch).
#[since(version = "0.10.1")]
pub trait BatchTransport<C, G>: Clone + OptionalSend + OptionalSync + 'static
where C: RaftTypeConfig
{
    /// Send `items` to `target`, and return their replies as the receiver produces them, in any
    /// order.
    ///
    /// An `Err` fails every item of the batch. An item without a reply when the stream ends fails
    /// with a network error. `option.hard_ttl()` is the time left for the whole exchange.
    fn send_batch(
        &self,
        target: C::NodeId,
        items: Vec<BatchItem<C, G>>,
        option: RPCOption,
    ) -> impl Future<Output = Result<BoxStream<'static, BatchReply<C, G>>, RPCError<C>>> + OptionalSend;
}
