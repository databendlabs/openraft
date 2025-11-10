# Distributed Key-Value Store with OpenRaft and gRPC

Demonstrates building a distributed key-value store with Openraft using gRPC for network communication.

## Key Features Demonstrated

- **gRPC networking**: Uses [Tonic](https://docs.rs/tonic) for Raft protocol and client communication
- **Protocol Buffers**: Type-safe RPC definitions for all network operations
- **Payload chunking**: Automatic retry with chunked transmission when append_entries exceeds gRPC message size limit
- **In-memory storage**: [`RaftLogStorage`] and protobuf-based state machine
- **Dual test modes**: Single-process cluster and multi-process realistic deployment

## Overview

This example implements:
- **Storage**: In-memory log storage with protobuf `StateMachineData` state machine
- **Network**: gRPC-based [`RaftNetwork`] implementation using Tonic
- **Services**: Separate gRPC services for application APIs and Raft internal communication

## Handling gRPC Payload Size Limits

Demonstrates automatic chunking when `append_entries` exceeds gRPC message size limits:

- **Error detection and retry**: [`src/network/mod.rs:137-145`](src/network/mod.rs#L137-L145)
- **Chunking logic**: [`src/network/mod.rs:63-125`](src/network/mod.rs#L63-L125)
- **Server size limit config**: [`src/app.rs:44-45`](src/app.rs#L44-L45)
- **Test**: Run `RUST_LOG=warn cargo test --test test_chunk -- --nocapture` to see chunking logs

## Testing Scenarios

**Single-process cluster** (`./tests/test_cluster.rs`):
- Brings up 3 nodes in one process
- Tests: initialize, add-learner, change-membership, write/read

**Chunking test** (`./tests/test_chunk.rs`):
- Demonstrates automatic payload chunking with 2-node cluster
- Uses large log entries to trigger gRPC size limit
- Verifies correct replication despite chunking

**Multi-process cluster** (`./test-cluster.sh`):
- Realistic 3-process deployment
- Same test sequence with actual network communication

## Architecture

**Key Code Locations**:
- Server entry point: `src/bin/main.rs`
- Network routing: `src/network/`
- gRPC services: `src/grpc/`
  - `api_service.rs` - Application APIs (read/write) and management
  - `raft_service.rs` - Raft internal protocol RPCs
- Protocol definitions: `protos/` - Protocol Buffer specifications
- Storage implementation: `src/store/mod.rs`

## Running

### Build

```shell
cargo build
```

### Start cluster

```bash
# Terminal 1
./raft-key-value --id 1 --addr 127.0.0.1:21001

# Terminal 2
./raft-key-value --id 2 --addr 127.0.0.1:21002

# Terminal 3
./raft-key-value --id 3 --addr 127.0.0.1:21003
```

### Initialize and test

1. Initialize first node as leader
2. Add learner nodes (2, 3)
3. Change membership to include all nodes
4. Write/read data via gRPC

See `./test-cluster.sh` for complete setup example.

## Comparison

| Feature | grpc | http |
|---------|------|------|
| Protocol | gRPC/Protobuf | HTTP/JSON |
| Type safety | Compile-time | Runtime |
| Performance | Higher | Good |
| Debugging | Harder | Easier |

Built for demonstration purposes.
