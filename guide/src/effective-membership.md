# Effective membership

In openraft a membership config log takes effect as soon as it is seen.

Thus, the **effective** membership is always the last present one found in log.

The effective membership is volatile before being committed:
because non-committed logs has chance being overridden by a new leader.
Thus, the effective membership needs to be reverted to the previous one,
i.e., the second last membership config log entry along with the conflicting
logs being deleted.

Because Raft does not allow to propose new membership config if the
effective one has not yet committed,
The Raft Engine only need to keep track of at most two membership configs: the last
committed one and the effective one.

The second last membership log has to be committed.

```rust
pub struct MembershipState<NID: NodeId> {
    pub committed: Arc<EffectiveMembership<NID>>,
    pub effective: Arc<EffectiveMembership<NID>>,
}
```

When deleting conflicting logs that contain a membership config log on a
Follower/Learner, it needs to revert at most one membership config to previous
one, i.e., discard the effective one and make the last committed one effective.
