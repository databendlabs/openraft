### What actions are required when a node restarts?

None. No calls, e.g., to either [`add_learner()`][] or [`change_membership()`][]
are necessary.

Openraft maintains the membership configuration in [`Membership`][] for all
nodes in the cluster, including voters and non-voters (learners).  When a
`follower` or `learner` restarts, the leader will automatically re-establish
replication.

An eligible former leader starts a higher-term election immediately after
restart when [`Config::enable_leader_restore`] is enabled, as it is by default. It
becomes leader only after winning the election quorum. With this setting
disabled, the node follows normal follower or learner startup and election
timing. A successful election appends a blank entry in the new term, so the log
index advances before the next client write.

If the state machine can lose applied state on restart, persist the committed
log ID through [`RaftLogStorage::save_committed`] or call
[`Raft::wait_for_recovery`] before serving reads.

[`add_learner()`]: `crate::Raft::add_learner`
[`change_membership()`]: `crate::Raft::change_membership`
[`Membership`]: `crate::Membership`
[`Config::enable_leader_restore`]: `field@crate::Config::enable_leader_restore`
[`RaftLogStorage::save_committed`]: `crate::storage::RaftLogStorage::save_committed`
[`Raft::wait_for_recovery`]: `crate::Raft::wait_for_recovery`
