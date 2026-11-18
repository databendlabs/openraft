# openraft-multi

Multi-Raft adapters for connection sharing across Raft groups.

## Components

- **`GroupRouter`** - Trait for sending RPCs with (target, group) routing
- **`GroupNetworkAdapter`** - Wraps `GroupRouter`, implements `RaftNetworkV2`
- **`GroupNetworkFactory`** - Simple factory + group_id wrapper
- **`BatchingRouter`** - Wraps a `GroupRouter` and sends the empty AppendEntries requests (heartbeats)
  of many groups to one node in shared batches, through a `BatchTransport`
- **`dispatch_batch()`** - Runs every item of a received batch against its local group

## Usage

1. Implement `GroupRouter` on your shared router/connection pool
2. Use `GroupNetworkAdapter` to get automatic `RaftNetworkV2` implementation

```rust
use openraft_multiraft::{GroupRouter, GroupNetworkAdapter, GroupNetworkFactory};

// Your Router implements GroupRouter
impl GroupRouter<TypeConfig, GroupId> for Router {
    type SnapshotData = SnapshotData;
    // ...
}

// Create factory for a specific group
let factory = GroupNetworkFactory::new(router, group_id);

// Factory creates adapters that implement RaftNetworkV2
impl RaftNetworkFactory<TypeConfig> for NetworkFactory {
    type Network = GroupNetworkAdapter<TypeConfig, GroupId, Router>;
    
    async fn new_client(&mut self, target: NodeId, _node: &Node) -> Self::Network {
        GroupNetworkAdapter::new(self.factory.clone(), target, self.group_id.clone())
    }
}
```

## Batching heartbeats

With many groups on one node, every leader sends its own heartbeat to every follower. To carry
the heartbeats bound for one node in fewer exchanges:

1. Implement `BatchTransport`, which sends a `Vec<BatchItem>` to one node and returns the replies
   as a stream. On the receiving node, pass each batch to `dispatch_batch()`, which calls
   `Raft::append_entries()` of each item's group.
2. Wrap your router: `BatchingRouter::new(router, transport, BatchConfig::default())`, and use it
   wherever you used the router.

Requests with entries, votes and snapshots still go through your router. Enable the `serde`
feature to serialize `BatchItem` and `BatchReply`.

## Examples

- [multi-raft-kv](../examples/multi-raft-kv/) - Basic Multi-Raft with 3 groups
