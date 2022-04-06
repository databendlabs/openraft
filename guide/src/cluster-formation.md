# Cluster Formation

A `Raft` node enters `Learner` state when it is created by `Raft::new()`.

To form a cluster, application must call `Raft::initialize(membership)`.


## `Raft::initialize()`

This method will:

- Append one membership log at index 0, the log id has to be `(leader_id=(0,0), index=0)`. 
  The membership will take effect at once.

- Enter Candidate state and start vote to become leader.

- The leader will commit a blank log to commit all preceding logs.


### Errors and failures

- Calling this method on an already initialized node just returns an error and is safe,
  i.e. `last_log_id` on this node is not None, or `vote` on this node is not `(0,0)`.

- Calling this method on more than one node at the same time:

    - with the same `membership`, it is safe.
      Because voting protocol guarantees consistency.

    - with different `membership` it is **ILLEGAL** and will result in an undefined
        state, AKA the **split-brain** state.


### Conditions for initialization

The conditions for a legal initialization is as the above because:

The first membership log with log id `(vote, index=0)` will be appended to initialize a node, without consensus.
This has not to break the commit condition:

1. Log id `(vote, index=0)` must not be greater than any committed log id.
   if `vote` is not the smallest value, i.e. `(term=0, node_id=0)`, it has chance to be greater than some
   committed log id. This is why the first log has to be the smallest: `((term=0, node_id=0), 0)`.

2. And a node should not append a log that is smaller than its `vote`.
   Otherwise, it is actually changing the **history** other nodes has seen.
   This has chance to (but not certainly will) break the consensus, depending on the protocol.
   E.g. if the cluster has been running a fast-paxos like protocol, appending a smaller log than `vote` is illegal.
   By not allowing to append a smaller log than `vote`, it will always be safe.

From these two reason, it is only allowed to append the first log if:
`vote==(0,0)`. And this is why the initial value of `vote` has to be `(0,0)`.
 