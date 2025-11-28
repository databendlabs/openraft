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
│   ├── network.rs      # Network implementation with group routing
│   ├── router.rs       # Message router for (node_id, group_id)
│   └── store.rs        # State machine storage
└── tests/
    └── cluster/
        ├── main.rs
        └── test_cluster.rs   # Integration tests
```
