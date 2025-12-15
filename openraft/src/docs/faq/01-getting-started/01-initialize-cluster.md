### How to initialize a cluster?

The simplest and most appropriate way to initialize a cluster is to call
`Raft::initialize()` on **exactly one node**. The other nodes should remain
empty and wait for the initialized node to replicate logs to them.

Assuming there are three nodes `n1, n2, n3`, there are two approaches:

1. **Single-step method**:
   Call `Raft::initialize()` on one node (e.g., `n1`) with the configuration of
   all three nodes: `n1.initialize(btreeset! {1,2,3})`.
   The initialized node will then replicate the membership to the other nodes.

2. **Incremental method**:
   First, call `Raft::initialize()` on `n1` with configuration containing only `n1`
   itself: `n1.initialize(btreeset! {1})`.
   Subsequently use `Raft::change_membership()` on `n1` to add `n2` and `n3`
   into the cluster.

The incremental method provides flexibility to start with a single-node
cluster for testing and expand it later for production.
