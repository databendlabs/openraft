# Effective membership

The membership config log in Openraft becomes **effective** upon being
viewed, resulting in the latest log being the effective membership.

As a result, the effective membership in Openraft is **unstable** until it is
committed, as uncommitted logs may be overwritten by a new leader. Therefore,
Openraft will revert membership config to the previous membership config log,
the second-to-last entry, when it conflicts with the leaders' logs.

Raft prohibits the proposal of a new membership config until the previous
one has been committed. Therefore, the Raft Engine only needs to maintain two
membership configurations at most: the last one that has been committed and the
effective one.

The second to last membership log has to be committed.

> An alternative solution is to make the membership config log effective only
> after it has been committed. However, this does not simplify the problem, as
> followers still need to keep track of the last two membership config logs.

In Openraft, the membership state contains these two configs:

```ignore
pub struct MembershipState<NID: NodeId> {
    pub committed: Arc<EffectiveMembership<NID>>,
    pub effective: Arc<EffectiveMembership<NID>>,
}
```

If conflicting logs that contain a membership configuration log need to be
deleted on a Follower/Learner, it is necessary to revert at most one membership
configuration to the previous one. This involves discarding the effective
configuration and making the last committed configuration effective.
