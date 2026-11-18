# Multi-Raft KV Store Example

This example demonstrates how to use OpenRaft's Multi-Raft support to run multiple independent Raft consensus groups within a single process.

## Overview

The example creates a distributed key-value store with **3 Raft groups**:
- **users** - Stores user data
- **orders** - Stores order data  
- **products** - Stores product data

Each group runs its own independent Raft consensus, but they share the same network infrastructure.

## Architecture

```
+-----------------------------------------------------------------------+
|                           Node 1                                       |
|  +-------------------+  +-------------------+  +-------------------+   |
|  |  Group "users"    |  |  Group "orders"   |  |  Group "products" |   |
|  |  (Raft Instance)  |  |  (Raft Instance)  |  |  (Raft Instance)  |   |
|  +-------------------+  +-------------------+  +-------------------+   |
|           |                      |                      |              |
|           +----------------------+----------------------+              |
|                                  |                                     |
|                         +--------+--------+                            |
|                         |     Router      |                            |
|                         | (shared network)|                            |
|                         +-----------------+                            |
+------------------------------------------------------------------------+
                                  |
                         Network Connection
                                  |
+------------------------------------------------------------------------+
|                               Node 2                                   |
|  +-------------------+  +-------------------+  +-------------------+   |
|  |  Group "users"    |  |  Group "orders"   |  |  Group "products" |   |
|  |  (Raft Instance)  |  |  (Raft Instance)  |  |  (Raft Instance)  |   |
|  +-------------------+  +-------------------+  +-------------------+   |
+------------------------------------------------------------------------+
```

## Key Concepts

### GroupId
A string identifier that uniquely identifies each Raft group (e.g., "users", "orders", "products").

### Shared Network
Multiple Raft groups share the same network infrastructure (`Router`), reducing connection overhead. Messages are routed to the correct group using the `group_id`.

### Independent Consensus
Each group runs its own Raft consensus independently:
- Separate log storage
- Separate state machine
- Separate leader election
- Separate membership

### Heartbeat Batching
Every group sends its own heartbeats, so with many groups most messages between two nodes are
AppendEntries requests without entries. This example sends them to each node in batches:

- Each node runs one timer, `send_heartbeats()` in `lib.rs`, that triggers the heartbeats of all
  its groups at once, with `enable_heartbeat` turned off in their `Config`. A group's own timer
  starts when it becomes leader, so heartbeats from separate timers would rarely meet in a batch.
- `NodeNetwork` sends every AppendEntries request without entries through the node's
  `AppendBatcher`. Its task collects the requests that arrive within 1 ms, and sends those for
  each target node as one `/raft/append-batch` message.
- The target node runs the requests against their groups concurrently, and replies with one
  result per request, in request order. A slow group delays the results of the others in its
  batch.

## Running the Test

```bash
# Run the integration test
cargo test -p multi-raft-kv test_multi_raft_cluster -- --nocapture

# With debug logging
RUST_LOG=debug cargo test -p multi-raft-kv test_multi_raft_cluster -- --nocapture
```

## Code Structure

```
multi-raft-kv/
├── Cargo.toml
├── README.md
├── src/
│   ├── lib.rs          # Type definitions and group constants
│   ├── app.rs          # Application handler for each group
│   ├── api.rs          # API handlers (read, write, raft operations)
│   ├── append_batcher.rs  # Batches empty AppendEntries requests by target node
│   ├── network.rs      # Network implementation with group routing
│   ├── router.rs       # Message router for (node_id, group_id)
│   └── store.rs        # State machine storage
└── tests/
    └── cluster/
        ├── main.rs
        └── test_cluster.rs   # Integration tests
```
