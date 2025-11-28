# Multi-Raft Sharding Example with TiKV-style Split

This example demonstrates how to implement **dynamic range sharding** with TiKV-style split using OpenRaft's Multi-Raft support.

## Overview

The example shows how to:
0. Suppose we have a "very hot" shard
1. We want to split it into two shards (atomic, no service interruption needed)
2. Migrate the new shard to dedicated nodes
3. Remove the migrated shard from original nodes

This is the same pattern used by TiKV for Region splits.

## Architecture

```
Phase 1: Initial State (3 nodes, 1 shard)
┌─────────────┐  ┌─────────────┐  ┌─────────────┐
│   Node 1    │  │   Node 2    │  │   Node 3    │
│ shard_a: ★  │  │ shard_a: F  │  │ shard_a: F  │
│ [1..200]    │  │ [1..200]    │  │ [1..200]    │
└─────────────┘  └─────────────┘  └─────────────┘

Phase 2: After Split (3 nodes, 2 shards)
┌─────────────┐  ┌─────────────┐  ┌─────────────┐
│   Node 1    │  │   Node 2    │  │   Node 3    │
│ shard_a: ★  │  │ shard_a: F  │  │ shard_a: F  │
│ [1..100]    │  │ [1..100]    │  │ [1..100]    │
│ shard_b: ★  │  │ shard_b: F  │  │ shard_b: F  │
│ [101..200]  │  │ [101..200]  │  │ [101..200]  │
└─────────────┘  └─────────────┘  └─────────────┘

Phase 3: Add Nodes 4,5 to shard_b
┌───────────┐ ┌───────────┐ ┌───────────┐ ┌───────────┐ ┌───────────┐
│  Node 1   │ │  Node 2   │ │  Node 3   │ │  Node 4   │ │  Node 5   │
│ shard_a:★ │ │ shard_a:F │ │ shard_a:F │ │     -     │ │     -     │
│ shard_b:★ │ │ shard_b:F │ │ shard_b:F │ │ shard_b:L │ │ shard_b:L │
└───────────┘ └───────────┘ └───────────┘ └───────────┘ └───────────┘

Phase 4: Migrate shard_b to Nodes 4,5
┌───────────┐ ┌───────────┐ ┌───────────┐ ┌───────────┐ ┌───────────┐
│  Node 1   │ │  Node 2   │ │  Node 3   │ │  Node 4   │ │  Node 5   │
│ shard_a:★ │ │ shard_a:F │ │ shard_a:F │ │     -     │ │     -     │
│     -     │ │     -     │ │     -     │ │ shard_b:★ │ │ shard_b:F │
└───────────┘ └───────────┘ └───────────┘ └───────────┘ └───────────┘

* is leader
F is follower
L is learner
```

## How Split Works

### Step 1: Propose Split as Raft Log

```rust
let split_request = Request::Split {
    split_at: 100,              // Split point
    new_shard_id: "shard_b",    // New shard ID
};
raft.client_write(split_request).await?;
```

### Step 2: State Machine Executes Split

When the split log is applied, each replica:

```rust
// In state machine apply():
Request::Split { split_at, new_shard_id } => {
    // 1. Extract data for new shard
    let split_data = self.keys_greater_than(*split_at);
    
    // 2. Remove extracted data from current shard  
    self.remove_keys_greater_than(*split_at);
    
    // 3. Return split data for bootstrapping new shard
    Response::SplitComplete {
        new_shard_id,
        split_data,
    }
}
```

### Step 3: Bootstrap New Shard

The split response contains the data for the new shard:

```rust
// Create new shard with split data
let state_machine = StateMachineStore::with_initial_data(split_data);
let raft_b = Raft::new(node_id, config, network, log_store, state_machine).await?;
```

## Running the Test

```bash
# Run the split test
cargo test -p multi-raft-sharding test_shard_split -- --nocapture

# With debug logging
RUST_LOG=debug cargo test -p multi-raft-sharding test_shard_split -- --nocapture
```

## Code Structure

```
multi-raft-sharding/
├── src/
│   ├── lib.rs          # Type definitions, new_raft()
│   ├── store.rs        # State machine with Split support
│   ├── shard_router.rs # Routes keys to shards by range
│   ├── router.rs       # Network message routing
│   ├── network.rs      # Raft network implementation
│   ├── app.rs          # Application message handler
│   └── api.rs          # API handlers
└── tests/
    └── cluster/
        └── test_split.rs  # Four-phase split test
```

## Use Cases

This pattern is useful for:

1. **Horizontal Scaling**: Add nodes for hot shards
2. **Load Balancing**: Distribute shards across nodes
3. **Data Locality**: Move shards closer to users
4. **Resource Isolation**: Dedicate nodes to critical shards

