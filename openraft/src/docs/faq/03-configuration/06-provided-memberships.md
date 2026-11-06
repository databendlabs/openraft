### How do I use a quorum rule other than a majority?

Select a membership from the [`openraft-membership`][] crate in your type
config. [`RaftTypeConfig::Membership`][] decides the read quorums that elect a
leader and the write quorums that commit an entry, and
[`RaftTypeConfig::MembershipEdit`][] decides how [`Raft::change_membership()`][]
describes a change. Both default to a majority: [`Membership`][] and
[`ChangeMembers`][].

The crate provides five memberships, each with its own change type:

| Membership | Change | A config sets |
|---|---|---|
| [`ThresholdMembership`][] | `ThresholdChange` | the voters and a write size; the read size is derived |
| [`WeightedMembership`][] | `WeightedChange` | a weight for each voter and a write weight |
| [`HierarchicalMembership`][] | `HierarchicalChange` | a tree of voter groups, such as zones, with a write size for each |
| [`GridMembership`][] | `GridChange` | a rectangular grid: a full row elects, a voter of each row commits |
| [`ExplicitMembership`][] | `ExplicitChange` | the minimal read quorums and the minimal write quorums |

Every membership makes each read quorum share a voter with each write quorum,
so a new leader holds every committed entry. Its [guide][`membership docs`]
compares them and explains when to pick each one.

**Steps**:

1. Add `openraft-membership` to your dependencies, with the same version as
   `openraft`. Keep its default `serde` feature on whenever `openraft/serde` is
   on anywhere in the build.

2. Select the membership and its change type:

   ```rust,ignore
   openraft::declare_raft_types!(
       pub TypeConfig:
           D = Request,
           R = Response,
           Membership = openraft_membership::WeightedMembership<u64, BasicNode>,
           MembershipEdit = openraft_membership::WeightedChange<Self::NodeId, Self::Node>,
   );
   ```

3. Build the first membership with its `new()` and pass it to
   [`Raft::initialize()`][]. A `BTreeMap` of nodes also converts into each
   membership and makes every node a voter, but the quorums differ: a majority
   write quorum for the threshold, weighted and hierarchical memberships, the
   most square grid for the grid membership, and every voter for the explicit
   membership:

   ```rust,ignore
   // Voter 1 weighs 2 of the total 5; a commit needs weight 3.
   let weights = BTreeMap::from([(1, 2), (2, 1), (3, 1), (4, 1)]);
   let membership = WeightedMembership::new(weights, 3, nodes)?;
   raft.initialize(membership).await?;
   ```

4. Change it with a value of the change type. OpenRaft writes the change in
   one log entry when it proves that every old read quorum intersects every
   new write quorum, and every old write quorum intersects every new read
   quorum. Otherwise it first writes a joint configuration:

   ```rust,ignore
   let change = WeightedChange::weights(BTreeMap::from([(1, 1), (2, 1), (3, 1), (4, 1)]));
   raft.change_membership(change, false).await?;
   ```

[`Raft::add_learner()`][] works unchanged with every provided membership.

The [`raft-kv-memstore-weighted-membership`][] example runs these steps over
HTTP with a `WeightedMembership`. Its `/init` and `/change-membership` routes
build the membership and the change from the weights in a JSON body, because
the membership routes of the other examples take only a set of voter IDs.

The weighted, hierarchical, grid and explicit memberships can also give a node
a role other than its default, for example a voter that votes and stores logs
but never starts an election. Set it with the membership's `with_roles()`, or
with `with_roles()` on a change.

To write a rule of your own, implement [`RaftMembership`][] and
[`RaftMembershipEdit`][]. The [weighted guide][`weighted membership`] walks
through how its types implement them.

[`openraft-membership`]: https://docs.rs/openraft-membership/latest/openraft_membership/
[`raft-kv-memstore-weighted-membership`]: https://github.com/databendlabs/openraft/tree/main/examples/raft-kv-memstore-weighted-membership
[`membership docs`]: https://docs.rs/openraft-membership/latest/openraft_membership/docs/index.html
[`weighted membership`]: https://docs.rs/openraft-membership/latest/openraft_membership/docs/weighted/index.html
[`ThresholdMembership`]: https://docs.rs/openraft-membership/latest/openraft_membership/docs/threshold/index.html
[`WeightedMembership`]: https://docs.rs/openraft-membership/latest/openraft_membership/docs/weighted/index.html
[`HierarchicalMembership`]: https://docs.rs/openraft-membership/latest/openraft_membership/docs/hierarchical/index.html
[`GridMembership`]: https://docs.rs/openraft-membership/latest/openraft_membership/docs/grid/index.html
[`ExplicitMembership`]: https://docs.rs/openraft-membership/latest/openraft_membership/docs/explicit/index.html
[`RaftTypeConfig::Membership`]: `crate::RaftTypeConfig::Membership`
[`RaftTypeConfig::MembershipEdit`]: `crate::RaftTypeConfig::MembershipEdit`
[`Membership`]: `crate::Membership`
[`ChangeMembers`]: `crate::impls::ChangeMembers`
[`RaftMembership`]: `crate::membership::RaftMembership`
[`RaftMembershipEdit`]: `crate::membership::RaftMembershipEdit`
[`Raft::initialize()`]: `crate::Raft::initialize`
[`Raft::change_membership()`]: `crate::Raft::change_membership`
[`Raft::add_learner()`]: `crate::Raft::add_learner`
