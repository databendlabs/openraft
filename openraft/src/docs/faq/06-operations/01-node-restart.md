### What actions are required when a node restarts?

None. No calls, e.g., to either [`add_learner()`][] or [`change_membership()`][]
are necessary.

Openraft maintains the membership configuration in [`Membership`][] for all
nodes in the cluster, including voters and non-voters (learners).  When a
`follower` or `learner` restarts, the leader will automatically re-establish
replication.

[`add_learner()`]: `crate::Raft::add_learner`
[`change_membership()`]: `crate::Raft::change_membership`
[`Membership`]: `crate::Membership`
