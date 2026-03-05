# OpenRaft Examples

This directory contains example applications demonstrating different implementation approaches for OpenRaft components.

## Complete Applications

### Component Overview

- **Log**: LogStore implementation for storing raft logs
- **State Machine**: StateMachine implementation for application state
- **RaftNetwork Impl**: Transport protocol and client library used
- **RaftNetwork**: Interface version (RaftNetwork vs RaftNetworkV2)
- **Client**: Client transport/library used by the example
- **Server**: Server runtime/framework used to accept requests
- **Special Features**: Unique characteristics of each example

| Example | Log | State Machine | RaftNetwork Impl | RaftNetwork | Client | Server | Special Features |
|---------|-----|---------------|------------------|-------------|--------|--------|------------------|
| [raft-kv-memstore] | [log-mem] | [sm-mem] | HTTP/reqwest | RaftNetwork | reqwest | actix-web | Basic example |
| [raft-kv-memstore-rkyv] | [log-mem] | in-memory (`rkyv`) | TCP + length-prefixed `rkyv` frames | RaftNetworkV2 | custom TCP (tests) | tokio `TcpListener` | custom wire protocol + `rkyv` serialization |
| [raft-kv-rocksdb] | [rocksstore] | [rocksstore] | HTTP/reqwest([network-v1]) | RaftNetwork | reqwest | actix-web | Persistent storage |
| [raft-kv-memstore-network-v2] | [log-mem] | [sm-mem] | HTTP/reqwest | RaftNetworkV2 | reqwest | actix-web | Network V2 interface |
| [multi-raft-kv] | [log-mem] | [sm-mem] | HTTP/channel | GroupRouter | channel | in-memory | Multi-Raft groups |
| [raft-kv-memstore-grpc] | [log-mem] | in-memory | gRPC/tonic | RaftNetwork | tonic | tonic | gRPC transport |
| [raft-kv-memstore-single-threaded] | [log-mem] | in-memory | HTTP/reqwest | RaftNetwork | reqwest | actix-web | Single-threaded runtime |
| [raft-kv-memstore-opendal-snapshot-data] | [log-mem] | in-memory+OpenDAL | HTTP/reqwest | RaftNetwork | reqwest | actix-web | OpenDAL snapshot storage |


## Component Implementations

### Storage Implementations
- **[log-mem]** - In-memory Raft Log Store using `std::collections::BTreeMap`
- **[sm-mem]** - In-memory KV State Machine implementation
- **[sm-mem-rkyv]** - In-memory KV State Machine implementation with `rkyv` snapshots
- **[rocksstore]** - RocksDB-based persistent storage using `rocksdb` crate

### Backward Compatibility (since 0.10)

The following symbolic links are provided for backward compatibility:

- **mem-log** → [log-mem] (renamed in 0.10)
- **memstore** → mem-log → [log-mem] (renamed in 0.9)

### Network Implementations
- **[network-v1]** - HTTP-based RaftNetwork interface V1 using `reqwest` crate

### Utilities
- **[types-kv]** - Shared KV request/response types for example crates
- **[utils]** - Shared type declarations and utilities

<!-- Reference Links -->
[raft-kv-memstore]: raft-kv-memstore/
[raft-kv-memstore-rkyv]: raft-kv-memstore-rkyv/
[raft-kv-rocksdb]: raft-kv-rocksdb/
[raft-kv-memstore-network-v2]: raft-kv-memstore-network-v2/
[raft-kv-memstore-grpc]: raft-kv-memstore-grpc/
[raft-kv-memstore-single-threaded]: raft-kv-memstore-single-threaded/
[raft-kv-memstore-opendal-snapshot-data]: raft-kv-memstore-opendal-snapshot-data/
[multi-raft-kv]: multi-raft-kv/
[log-mem]: log-mem/
[sm-mem]: sm-mem/
[sm-mem-rkyv]: sm-mem-rkyv/
[rocksstore]: rocksstore/
[network-v1]: network-v1-http/
[types-kv]: types-kv/
[utils]: utils/

[memstore]: memstore/
