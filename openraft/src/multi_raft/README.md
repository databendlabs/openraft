## Multi-Raft Support

Adapters for connection sharing across multiple Raft groups:

- **`GroupedRpc`** - Trait for sending RPCs with group routing
- **`GroupNetworkAdapter`** - Wraps `GroupedRpc`, implements `RaftNetworkV2`
- **`GroupNetworkFactory`** - Simple factory + group_id wrapper

### Usage

1. Implement `GroupedRpc` on your shared connection
2. Wrap with `GroupNetworkAdapter` to get automatic `RaftNetworkV2`

**Note:** Disable `adapt-network-v1` feature to use `GroupNetworkAdapter`:
```toml
openraft = { version = "0.10", default-features = false, features = ["multi-raft", "tokio-rt"] }
```

### Examples

- [multi-raft-kv](../../../examples/multi-raft-kv/) - Basic Multi-Raft with 3 groups
- [multi-raft-sharding](../../../examples/multi-raft-sharding/) - TiKV-style shard split
