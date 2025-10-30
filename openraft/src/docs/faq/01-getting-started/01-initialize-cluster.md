### How to initialize a cluster?

There are two ways to initialize a raft cluster, assuming there are three nodes,
`n1, n2, n3`:

1. Single-step method:
   Call `Raft::initialize()` on any one of the nodes with the configuration of
   all three nodes, e.g. `n2.initialize(btreeset! {1,2,3})`.

2. Incremental method:
   First, call `Raft::initialize()` on `n1` with configuration containing `n1`
   itself, e.g., `n1.initialize(btreeset! {1})`.
   Subsequently use `Raft::change_membership()` on `n1` to add `n2` and `n3`
   into the cluster.

Employing the second method provides the flexibility to start with a single-node
cluster for testing purposes and subsequently expand it to a three-node cluster
for deployment in a production environment.
