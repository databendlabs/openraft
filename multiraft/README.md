# openraft-multi

Multi-Raft adapters for connection sharing across Raft groups.

## Components

- **`GroupedRpc`** - Trait for sending RPCs with (target, group) routing
- **`GroupNetworkAdapter`** - Wraps `GroupedRpc`, implements `RaftNetworkV2`
- **`GroupNetworkFactory`** - Simple factory + group_id wrapper

## Usage

1. Implement `GroupedRpc` on your shared router/connection pool
2. Use `GroupNetworkAdapter` to get automatic `RaftNetworkV2` implementation

```rust
use openraft_multiraft::{GroupedRpc, GroupNetworkAdapter, GroupNetworkFactory};

// Your Router implements GroupedRpc
impl GroupedRpc<TypeConfig, GroupId> for Router {
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

## Examples

- [multi-raft-kv](../examples/multi-raft-kv/) - Basic Multi-Raft with 3 groups
- [multi-raft-sharding](../examples/multi-raft-sharding/) - TiKV-style shard split

