# Formation of Clusters

When a [`Raft`] node is created by [`Raft::new()`], it enters the [`Learner`] state.

To establish a cluster, invoke [`Raft::initialize()`].

[`Raft`]: `crate::Raft`
[`Learner`]: `crate::core::ServerState::Learner`


## [`Raft::initialize()`]

This method will:

- Add a membership log at index 0. Its log ID has the default term and this node's ID, as the
  committed leader ID type keeps them: with advanced leader IDs, node 1 appends log
  `(term=0, node_id=1, index=0)`; a standard committed leader ID keeps only the term.
  The membership is effective at once. `initialize()` returns once this log is flushed to this
  node's storage; the elected leader commits it later.

- Transition to the [`Candidate`] state and initiate voting to become the leader.

[`Candidate`]: `crate::core::ServerState::Candidate`

- The leader will commit a blank log to commit all previous logs.

### Recommended Usage

The simplest and most appropriate way to initialize a cluster is to call `initialize()`
on **exactly one node**. The other nodes should remain empty and wait for the initialized
node to replicate logs to them.

Calling `initialize()` on multiple nodes with **identical configuration** is also
acceptable and will not cause any consistency issues — the Raft voting protocol ensures
that only one leader will be elected.

However, calling `initialize()` with **different configurations** on different nodes
may lead to a split-brain condition and must be avoided.

### Errors and Failures

- If this method is called on a node that has already been initialized, it will simply return an error and remain safe,
  i.e., if this node has a log, or its vote has a term other than the default term.

- It also returns an error if this node cannot campaign in the given membership, as a voter of a
  default membership can, or if the membership is not a valid initial config.

### Preconditions for Initialization

`initialize()` appends the initial membership log without consensus, so it is allowed only when
this node has no log and its vote has the default term. The node ID of the vote is not checked: a
node that never voted holds a vote for itself with the default term.

These conditions keep the appended log from violating the commit condition:

1. The first log must not exceed any committed log ID.
   A log is committed only through an elected leader, whose term is greater than the default
   term, and log IDs compare by term first. So the first log, which has the default term, is
   below every committed log ID, whatever node ID its leader ID holds.

2. A node should not append a log that is smaller than its `vote`.
   Otherwise, it is effectively altering the **history** seen by other nodes.
   This may (but not necessarily) disrupt consensus, depending on the protocol.
   For example, if the cluster has been running a fast-paxos-like protocol, appending a smaller log than `vote` is illegal.
   By prohibiting the appending of a smaller log than `vote`, it will always be safe.

   A node whose vote has a term greater than the default term may not initialize for this
   reason. The initializing node commits a vote for the leader ID of the first log, which has
   the default term and this node's ID, so the log is not smaller than its vote.

[`Raft::initialize()`]: `crate::Raft::initialize`
[`Raft::new()`]:        `crate::Raft::new`
