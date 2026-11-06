use openraft::RaftTypeConfig;
use openraft::alias::DefaultEntryOf;
use openraft::alias::EntryPayloadOf;
use types_kv::Request;
use types_kv::Response;

/// A type config that the cluster's stores accept: `types_kv` requests and responses, `u64` node
/// IDs, `()` node metadata, and the default payload and entry types.
///
/// Each scenario's type config adds its membership and its edit type.
pub trait KvTypeConfig
where Self: RaftTypeConfig<
            D = Request,
            R = Response,
            NodeId = u64,
            Node = (),
            Payload = EntryPayloadOf<Self>,
            Entry = DefaultEntryOf<Self>,
        >
{
}

impl<C> KvTypeConfig for C where C: RaftTypeConfig<
            D = Request,
            R = Response,
            NodeId = u64,
            Node = (),
            Payload = EntryPayloadOf<C>,
            Entry = DefaultEntryOf<C>,
        >
{
}
