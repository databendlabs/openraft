# Node Lifecycle

- When a node is added using [`Raft::add_learner()`], it immediately starts receiving log
  replication from the leader, becoming a [`Learner`].

- A [`Learner`] transitions to a voter when [`Raft::change_membership()`] includes it as a
  voter. A voter can be in [`Follower`], [`Candidate`], or [`Leader`] state.

[`Learner`]: `crate::core::ServerState::Learner`
[`Follower`]: `crate::core::ServerState::Follower`
[`Candidate`]: `crate::core::ServerState::Candidate`
[`Leader`]: `crate::core::ServerState::Leader`

- When a node, regardless of being a [`Learner`] or voter, is removed from membership, the
  leader ceases replication to it immediately when the new membership that
  excludes the node is observed (no need for commitment).

  The removed node will not receive any log replication or heartbeat from the
  leader. It will enter the [`Candidate`] state because it is unaware of its removal.


## Removing a Node from Membership Config

When membership changes, for example, from a joint config `[(1,2,3),
(3,4,5)]` to a uniform config `[3,4,5]` (assuming the leader is `3`), the leader
stops replication to `1,2` when the uniform config `[3,4,5]` is observed (no need for commitment).

This is correct because:

- If the leader (`3`) eventually commits `[3,4,5]`, it will ultimately stop replication to `1,2`.

- If the leader (`3`) crashes before committing `[3,4,5]`:
    - And a new leader observes the membership config log `[3,4,5]`, it will proceed to commit it and finally stop replication to `1,2`.
    - Or a new leader does not observe the membership config log `[3,4,5]`, it will re-establish replication to `1,2`.

In any scenario, stopping replication immediately is acceptable.

One consideration is that nodes, such as `1,2`, are unaware of their removal from the cluster:

- Removed nodes will enter the [`Candidate`] state and continue to increase their `term` and elect themselves.
  This will not impact the functioning cluster:

    - The nodes in the functioning cluster have more logs; thus, the election will never succeed.

    - The leader will not attempt to communicate with the removed nodes, so it will not see their higher `term`.

- Removed nodes should eventually be shut down. Regardless of whether the leader
  replicates the membership without these removed nodes to them, an external process should
  always shut them down. This is because there is no
  guarantee that a removed node can receive the membership log within a finite time.


[`Raft::add_learner()`]: `crate::Raft::add_learner`
[`Raft::change_membership()`]: `crate::Raft::change_membership`
[`extended_membership`]: `crate::docs::data::extended_membership`
