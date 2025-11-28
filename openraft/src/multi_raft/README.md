## When to Use Multi-Raft

Multi-Raft is useful when you need to:

1. **Shard data** - Distribute data across multiple independent groups
2. **Reduce consensus overhead** - Smaller groups = faster consensus
3. **Isolate failures** - Issues in one group don't affect others
4. **Scale horizontally** - Add more groups as data grows

## Comparison with Single-Raft

| Aspect | Single-Raft | Multi-Raft |
|--------|-------------|------------|
| Consensus scope | All data | Per-group data |
| Leader election | One leader | One leader per group |
| Log replication | Single stream | Per-group streams |
| Failure isolation | All-or-nothing | Per-group |
| Network connections | One per peer | Shared across groups |


## Examples

Two complete examples demonstrate Multi-Raft usage:

### [multi-raft-kv](../../../examples/multi-raft-kv/)

A basic Multi-Raft example with **3 independent KV groups** (`users`, `orders`, `products`) running on multiple nodes.

**Key features demonstrated:**
- Multiple Raft groups on a single node
- Independent leader election per group
- Leader distribution across different nodes
- Data isolation between groups

### [multi-raft-sharding](../../../examples/multi-raft-sharding/)

An advanced example demonstrating **TiKV-style shard split and migration**.

**Key features demonstrated:**
- Atomic shard split as a Raft log entry
- Data migration without service interruption
- Dynamic membership changes
- Adding new nodes and migrating shards

**Scenario:**
1. Start with 3 nodes, 1 shard containing all users
2. Split shard at user_id=100 (atomic operation)
3. Add new nodes (4, 5) and replicate the new shard
4. Migrate the new shard to nodes 4, 5 only