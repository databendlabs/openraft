# OpenRaft Examples

This directory contains example applications demonstrating different implementation approaches for OpenRaft components.

## Complete Applications

Start with [raft-kv-memstore]: it is the canonical example application, and the
[getting-started guide](https://docs.rs/openraft/latest/openraft/docs/getting_started/index.html)
follows it step by step. Every other row below varies one component of that
application — storage, network interface, transport, or runtime — and is worth
reading once the canonical one makes sense.

### Component Overview

- **Log**: LogStore implementation for storing raft logs
- **State Machine**: StateMachine implementation for application state
- **RaftNetwork Impl**: Transport protocol and client library used
- **RaftNetwork**: Interface version (RaftNetwork vs RaftNetworkV2)
- **Client**: HTTP/gRPC client library for application requests
- **Server**: HTTP server implementation for handling incoming requests
- **Special Features**: Unique characteristics of each example

| Example | Log | State Machine | RaftNetwork Impl | RaftNetwork | Client | Server | Special Features |
|---------|-----|---------------|------------------|-------------|--------|--------|------------------|
| [raft-kv-memstore] | [log-mem] | [sm-mem] | HTTP/reqwest([network-v2]) | RaftNetworkV2 | [app-http] | [app-http] | Canonical example — start here |
| [raft-kv-rocksdb] | [log-rocks] | RocksDB | HTTP/reqwest([network-v2]) | RaftNetworkV2 | [app-http] | [app-http] | [raft-kv-memstore] with persistent storage |
| [raft-kv-memstore-network-v1] | [log-mem] | [sm-mem] | HTTP/reqwest([network-v1]) | RaftNetwork | [app-http] | [app-http] | Legacy V1 network + chunked snapshot replication |
| [multi-raft-kv] | [log-mem] | [sm-mem] | HTTP/channel | GroupRouter | channel | in-memory | Multi-Raft groups |
| [raft-kv-memstore-grpc] | [log-mem] | in-memory | gRPC/tonic | RaftNetworkV2 sub-traits | tonic | tonic | gRPC transport |
| [raft-kv-memstore-single-threaded] | in-memory | in-memory | in-process channel | RaftNetworkV2 | channel | in-memory | Single-threaded runtime |
| [raft-kv-memstore-opendal-snapshot-data] | [log-mem] | in-memory+OpenDAL | in-process channel | RaftNetworkV2 | channel | in-memory | OpenDAL snapshot storage |


## Component Implementations

### Storage Implementations
- **[log-mem]** - In-memory Raft Log Store using `std::collections::BTreeMap`
- **[log-rocks]** - RocksDB-based persistent Raft Log Store
- **[sm-mem]** - In-memory KV State Machine implementation
- **[sm-rocks]** - RocksDB-based persistent state machine

Performance note: Raft log workloads are mostly append-only. RocksDB's general-purpose LSM design
adds compaction and write-amplification overhead, so [log-rocks] is a durable example rather than an
optimal-performance log store.

### Backward Compatibility (since 0.10)

The following symbolic links are provided for backward compatibility:

- **mem-log** → [log-mem] (renamed in 0.10)
- **memstore** → mem-log → [log-mem] (renamed in 0.9)
- **rocksstore** → [sm-rocks] (renamed in 0.10)

### Network Implementations
- **[network-v2]** - HTTP-based RaftNetworkV2 interface using `reqwest` crate
- **[network-v1]** - HTTP-based RaftNetwork interface V1 using `reqwest` crate

### Application HTTP
- **[app-http]** - JSON HTTP client and server used by the KV examples

### Utilities
- **[types-kv]** - Shared KV request/response types for example crates
- **[utils]** - Shared type declarations and utilities

<!-- Reference Links -->
[raft-kv-memstore]: raft-kv-memstore/
[raft-kv-rocksdb]: raft-kv-rocksdb/
[raft-kv-memstore-network-v1]: raft-kv-memstore-network-v1/
[raft-kv-memstore-grpc]: raft-kv-memstore-grpc/
[raft-kv-memstore-single-threaded]: raft-kv-memstore-single-threaded/
[raft-kv-memstore-opendal-snapshot-data]: raft-kv-memstore-opendal-snapshot-data/
[multi-raft-kv]: multi-raft-kv/
[log-mem]: log-mem/
[log-rocks]: log-rocks/
[sm-mem]: sm-mem/
[sm-rocks]: sm-rocks/
[network-v2]: network-v2-http/
[network-v1]: network-v1-http/
[app-http]: app-http/
[types-kv]: types-kv/
[utils]: utils/

[memstore]: memstore/
