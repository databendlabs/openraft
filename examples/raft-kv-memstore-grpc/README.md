# Distributed Key-Value Store with OpenRaft and gRPC

Demonstrates building a distributed key-value store with Openraft using gRPC for network communication.

## Key Features Demonstrated

- **gRPC networking**: Uses [Tonic](https://docs.rs/tonic) for Raft protocol and client communication
- **Protocol Buffers**: Type-safe RPC definitions for all network operations
- **Sub-trait implementation**: Implements individual `Net*` sub-traits directly instead of `RaftNetworkV2`
- **Pipeline streaming**: Bidirectional gRPC streaming for efficient log replication via `NetStreamAppend`
- **Heartbeats on their own connection**: `Network` keeps one channel per peer for replication,
  votes and snapshots, and a second one for heartbeats, through `new_heartbeat_client()`
- **RPC deadlines**: Votes, pre-votes and opening the append stream end at `RPCOption::soft_ttl()`
- **In-memory storage**: [`RaftLogStorage`] and protobuf-based state machine

## Overview

This example implements:
- **Storage**: In-memory log storage with protobuf `StateMachineData` state machine
- **Network**: Implements `NetStreamAppend`, `NetVote`, `NetSnapshot`, `NetBackoff`, and
  `NetTransferLeader` sub-traits directly (instead of implementing `RaftNetworkV2` as a
  monolithic trait). This approach is suited for gRPC where native bidirectional streaming
  maps naturally to `NetStreamAppend`.
- **Services**: Separate gRPC services for application APIs and Raft internal communication

> **Note**: For most applications, implementing [`RaftNetworkV2`] is recommended for simplicity.
> This example demonstrates the advanced sub-trait approach to showcase fine-grained control
> over network capabilities.

## Connections and deadlines

Each channel is one HTTP/2 connection, which tonic opens on first use and opens again after a
failure. Heartbeats get a channel of their own because HTTP/2 flow control limits a whole
connection as well as each stream on it. A follower whose log flush is slow stops reading the
replication stream, and the data it has not read stays counted against the connection's window.
On a shared connection, a heartbeat would then wait until the follower reads again, although the
follower answers a heartbeat at once: a heartbeat waits for no log flush.

The answers on an open append stream have no deadline, because each one waits for the follower's
log flush, which may take long while nothing is wrong. Measure heartbeat latency apart from append
and flush latency before choosing timeouts: `tests/heartbeat_during_slow_flush.rs` prints both.

## Testing

**Single-process cluster** (`./tests/test_cluster.rs`):
- Brings up 3 nodes in one process
- Tests: initialize, add-learner, change-membership, write/read

**Slow follower** (`./tests/heartbeat_during_slow_flush.rs`):
- A learner whose log store holds each flush for 5 s keeps answering the leader's heartbeats
- Prints the heartbeat round trips, apart from how long the appends took to be answered

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
- Protocol definitions: `proto/` - Protocol Buffer specifications
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
