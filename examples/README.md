# OpenRaft Examples

This directory contains example applications demonstrating different implementation approaches for OpenRaft components.

## Complete Applications

### Component Overview

- **Log**: LogStore implementation for storing raft logs
- **State Machine**: StateMachine implementation for application state
- **RaftNetwork Impl**: Transport protocol and client library used
- **RaftNetwork**: Interface version (RaftNetwork vs RaftNetworkV2)
- **Client**: HTTP/gRPC client library for application requests
- **Server**: Web framework for handling incoming requests
- **Special Features**: Unique characteristics of each example

| Example | Log | State Machine | RaftNetwork Impl | RaftNetwork | Client | Server | Special Features |
|---------|-----|---------------|------------------|-------------|--------|--------|------------------|
| [raft-kv-memstore] | [memstore] | in-memory | HTTP/reqwest | RaftNetwork | reqwest | actix-web | Basic example |
| [raft-kv-rocksdb] | [rocksstore] | [rocksstore] | HTTP/reqwest([network-v1]) | RaftNetwork | reqwest | actix-web | Persistent storage |
| [raft-kv-memstore-network-v2] | [memstore] | in-memory | HTTP/reqwest | RaftNetworkV2 | reqwest | actix-web | Network V2 interface |
| [raft-kv-memstore-grpc] | [memstore] | in-memory | gRPC/tonic | RaftNetwork | tonic | tonic | gRPC transport |
| [raft-kv-memstore-singlethreaded] | memstore | in-memory | HTTP/reqwest | RaftNetwork | reqwest | actix-web | Single-threaded runtime |
| [raft-kv-memstore-opendal-snapshot-data] | [memstore] | in-memory+OpenDAL | HTTP/reqwest | RaftNetwork | reqwest | actix-web | OpenDAL snapshot storage |


## Component Implementations

### Storage Implementations
- **[memstore]** - In-memory LogStore and StateMachine using `std::collections::HashMap` and `Vec`
- **[rocksstore]** - RocksDB-based persistent storage using `rocksdb` crate

### Network Implementations
- **[network-v1]** - HTTP-based RaftNetwork interface V1 using `reqwest` crate

### Utilities
- **[utils]** - Shared type declarations and utilities

<!-- Reference Links -->
[raft-kv-memstore]: raft-kv-memstore/
[raft-kv-rocksdb]: raft-kv-rocksdb/
[raft-kv-memstore-network-v2]: raft-kv-memstore-network-v2/
[raft-kv-memstore-grpc]: raft-kv-memstore-grpc/
[raft-kv-memstore-singlethreaded]: raft-kv-memstore-singlethreaded/
[raft-kv-memstore-opendal-snapshot-data]: raft-kv-memstore-opendal-snapshot-data/
[memstore]: memstore/
[rocksstore]: rocksstore/
[network-v1]: network-v1/
[utils]: utils/
