# Cluster Formation

A `Raft` node enters `Learner` state when it is created by `Raft::new()`.

To form a cluster, application must call `Raft::initialize(membership)`.


## `Raft::initialize()`

This method will:

- Update the in-memory membership.

- If there are more than one voters in the config, enter Candidate state and start vote to become leader.

- If there is only one voters(itself) in the config, become leader at once and
    will enter leader state.

- The leader will commit a log contains the given membership config.

TODO(xp): this procedure will be changed:  https://github.com/datafuselabs/openraft/issues/45


- Calling this method on an already initialized node just returns an error and is safe.

- Calling this method on more than one node at the same time:

    - with the same `membership` is safe.
      Because voting protocol guarantees consistency.

    - with different `membership` is **ILLEGAL** and will result in a undefined
        state, AKA the **split-brain** state.
