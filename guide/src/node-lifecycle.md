# Node life cycle

- When a node is added with `Raft::add_learner()`, it starts to receive log
    replication from the leader at once, i.e., becomes a `Learner`.

- A learner becomes a `Voter`, when `Raft::change_membership()` adds it a
    `Voter`. A `Voter` will then become `Candidate` or `Leader`.

- When a node, no matter a `Learner` or `Voter`, is removed from membership, the
    leader stops replicating to it at once, i.e., when the new membership that
    does not contain the node is seen(no need to commit).

    The removed node won't receive any log replication or heartbeat from the
    leader. It will enter `Candidate` because it does not know it is removed.


## Remove a node from membership config

When membership changes, e.g., from a joint config `[(1,2,3),
(3,4,5)]` to uniform config `[3,4,5]`(assuming the leader is `3`), the leader
stops replication to `1,2` when `[3,4,5]` is seen(not committed).

It is correct because:

- If the leader(`3`) finally committed `[3,4,5]`, it will eventually stop replication to `1,2`.

- If the leader(`3`) crashes before committing `[3,4,5]`:
  - And a new leader sees the membership config log `[3,4,5]`, it will continue to commit it and finally stop replication to `1,2`.
  - Or a new leader does not see membership config log `[3,4,5]`, it will re-establish replication to `1,2`.

In any case, stopping replication at once is OK.

One of the considerations is:
The nodes, e.g., `1,2` do not know they have been removed from the cluster:

- Removed node will enter the candidate state and keeps increasing its term and electing itself.
  This won't affect the working cluster: 

  - The nodes in the working cluster have greater logs; thus, the election will never succeed.

  - The leader won't try to communicate with the removed nodes thus it won't see their higher `term`.

- Removed nodes should be shut down finally. No matter whether the leader
  replicates the membership without these removed nodes to them, there should
  always be an external process that shuts them down. Because there is no
  guarantee that a removed node can receive the membership log in a finite time.



