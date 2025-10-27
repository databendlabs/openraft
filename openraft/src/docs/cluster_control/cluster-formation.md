# Formation of Clusters

When a [`Raft`] node is created by [`Raft::new()`], it enters the [`Learner`] state.

To establish a cluster, invoke [`Raft::initialize()`].

[`Raft`]: `crate::Raft`
[`Learner`]: `crate::core::ServerState::Learner`


## [`Raft::initialize()`]

This method will:

- Add a membership log at index 0 with the log id `(leader_id=LeaderId::default(), index=0)`.
  The membership will be immediately effective.

- Transition to the [`Candidate`] state and initiate voting to become the leader.

[`Candidate`]: `crate::core::ServerState::Candidate`

- The leader will commit a blank log to commit all previous logs.

### Errors and Failures

- If this method is called on a node that has already been initialized, it will simply return an error and remain safe,
  i.e., if `last_log_id` on this node is not `None`, or `vote` on this node is not `(0,0)`.

- If this method is called on more than one node simultaneously:

    - with the same `membership`, it is safe,
      as the voting protocol ensures consistency.

    - with different `membership`, it is **ILLEGAL** and will lead to an undefined
      state, also known as the **split-brain** state.

### Preconditions for Initialization

The legal initialization conditions are as follows:

The initial membership log with log id `(leader_id=LeaderId::default(), index=0)` will be appended to a node without consensus.
This must not violate the commit condition:

1. Log id `(leader_id=LeaderId::default(), index=0)` must not exceed any committed log id.
   If `leader_id` is not the smallest value, i.e., `(term=0, node_id=0)`, it may be greater than some
   committed log id. This is why the first log must be the smallest: `((term=0, node_id=0), 0)`.

2. A node should not append a log that is smaller than its `vote`.
   Otherwise, it is effectively altering the **history** seen by other nodes.
   This may (but not necessarily) disrupt consensus, depending on the protocol.
   For example, if the cluster has been running a fast-paxos-like protocol, appending a smaller log than `vote` is illegal.
   By prohibiting the appending of a smaller log than `vote`, it will always be safe.

For these two reasons, appending the first log is only allowed if:
`vote==(0,0)`. This is why the initial value of `vote` must be `(0,0)`.

[`Raft::initialize()`]: `crate::Raft::initialize`
[`Raft::new()`]:        `crate::Raft::new`
