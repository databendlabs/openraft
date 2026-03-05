# Distributed Key-Value Store with OpenRaft over TCP + rkyv

Demonstrates a distributed key-value store built on OpenRaft using a custom TCP transport and `rkyv`-serialized wire messages.

## Key Features Demonstrated

- **Custom TCP networking**: Raft RPCs are exchanged over `tokio::net::TcpStream`.
- **Length-prefixed framing**: Each request/response is sent as `u32` length + payload bytes.
- **`rkyv` serialization**: Wire RPC enums and snapshot/state-machine data use zero-copy-friendly `rkyv` formats.
- **`RaftNetworkV2` implementation**: Implements `append_entries`, `vote`, `full_snapshot`, `transfer_leader`, and `backoff`.
- **In-memory storage**: `log-mem::LogStore` plus an in-memory `StateMachineStore` with snapshot support.

## Overview

This example includes:

- **Storage**:
  - In-memory log store (`log-mem`)
  - In-memory state machine (`BTreeMap<String, String>`) and snapshot build/install logic in `src/store/mod.rs`
- **Network**:
  - A TCP-based `RaftNetworkFactory` and `RaftNetworkV2` implementation in `src/network/mod.rs`
  - Request/response wire enums (`WireRequest`, `WireResponse`) serialized with `rkyv`
- **Application process**:
  - Node bootstrap in `src/app.rs`
  - Binary entrypoint and CLI args in `src/bin/main.rs`

## Testing

- **Storage conformance** (`src/test_store.rs`):
  - Runs OpenRaft's storage test suite against this example's in-memory store.
- **Message serialization tests** (`tests/test_rkyv_messages.rs`):
  - Validates `rkyv` round-trip for vote/append/snapshot/transfer-leader/write message types.
- **Single-process transport test** (`tests/test_cluster.rs`):
  - Spawns 3 nodes and verifies framed TCP Raft RPC behavior.
- **Multi-process smoke test** (`./test-cluster.sh`):
  - Builds the binary, starts 5 local nodes, probes TCP endpoints, and tails recent logs.

## Architecture

**Key code locations:**

- `src/bin/main.rs`: CLI and runtime bootstrap.
- `src/app.rs`: Raft config, store/network setup, and TCP listener loop.
- `src/network/mod.rs`: TCP wire protocol, framing, request handling, and `RaftNetworkV2`.
- `src/store/mod.rs`: State machine and snapshot implementation.
- `src/raft.rs`: App data types (`SetRequest`, `Response`, `Node`, `Vote`, `Entry`).
- `tests/`: integration tests for transport and serialization.

## Running

### Build

```shell
cargo build
```

### Run tests

```shell
cargo test
```

### Start a 3-node cluster manually

```bash
# Terminal 1
./target/debug/raft-key-value --id 1 --addr 127.0.0.1:5051

# Terminal 2
./target/debug/raft-key-value --id 2 --addr 127.0.0.1:5052

# Terminal 3
./target/debug/raft-key-value --id 3 --addr 127.0.0.1:5053
```

### Multi-process smoke test

```bash
./test-cluster.sh
```
