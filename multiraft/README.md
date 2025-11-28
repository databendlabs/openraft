# openraft-multi

Multi-Raft adapters for connection sharing across Raft groups.

## Components

- **`GroupRouter`** - Trait for sending RPCs with (target, group) routing
- **`GroupNetworkAdapter`** - Wraps `GroupRouter`, implements `RaftNetworkV2`
- **`GroupNetworkFactory`** - Simple factory + group_id wrapper

## Usage

1. Implement `GroupRouter` on your shared router/connection pool
2. Use `GroupNetworkAdapter` to get automatic `RaftNetworkV2` implementation

```rust
use openraft_multiraft::{GroupRouter, GroupNetworkAdapter, GroupNetworkFactory};

// Your Router implements GroupRouter
impl GroupRouter<TypeConfig, GroupId> for Router {
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

