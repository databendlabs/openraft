# Provided memberships: pick how each config states its quorums

Every membership in this crate gives each config read quorums and write
quorums. An entry commits once a write quorum stores it, and a candidate
becomes leader only with votes from a read quorum. Every read quorum shares a
voter with every write quorum, so a new leader holds every committed entry.
The memberships differ in how a config states its quorums. That decides how
many voters a commit and an election wait for, and which failures stop the
cluster:

| Membership                 | A config sets                                                                     | Pick it when                                                                                             |
|----------------------------|-----------------------------------------------------------------------------------|----------------------------------------------------------------------------------------------------------|
| [`ThresholdMembership`]    | the voters and a write size                                                       | every voter counts the same, and you choose how many copies a commit needs                               |
| [`WeightedMembership`]     | a weight for each voter and a write weight                                        | some voters, or the sites that hold them, must count more                                                |
| [`HierarchicalMembership`] | voter groups, such as zones, and how many voters in how many groups a write needs | the cluster must keep running after it loses a whole zone                                                |
| [`GridMembership`]         | a rectangular grid of voters                                                      | a commit must wait for fewer voters than a majority, and so must an election with the default leader IDs |
| [`ExplicitMembership`]     | the minimal read quorums and the minimal write quorums                            | no rule above states the policy, such as a backup voter in every write quorum and in no read quorum      |

The guides [`threshold`], [`weighted`], [`hierarchical`], [`grid`] and
[`explicit`] each show how to configure the membership, why its read and
write quorums intersect, what a commit and an election wait for, which
failures stop it, and how to change it. [`weighted`] also walks through how
its quorum rule, its membership and its change implement the traits that
OpenRaft calls, and what to copy to write a membership of your own. The
hierarchical, grid and explicit guides name what their types do differently.

## A voter with only `ReceiveLogs` votes and stores logs, but starts no election

The weighted, hierarchical, grid and explicit memberships let a config give a
node a [`NodeRole`] other than its default. [`NodeRole::Campaigner`] lets a
node start an election, and [`NodeRole::ReceiveLogs`] lets it receive logs.
By default a voter has both, and every other node has only `ReceiveLogs`.
[`ThresholdMembership`] always uses the defaults.

A voter with only `ReceiveLogs` stores logs and votes, but never starts an
election on its own. Use it for a voter that should not lead, such as one in a
remote site. It can still become leader when the leader transfers leadership to
it with [`Trigger::transfer_leader()`]: a transfer starts an election on the
target even without `Campaigner`.

Each of the four memberships sets roles with its `with_roles()` method, such
as [`WeightedMembership::with_roles()`]. Each change type sets them with its
`roles` field or its `with_roles()` builder. A change that sets only roles
keeps every quorum, so OpenRaft appends it without a joint config:

```rust,ignore
// Voter 3 keeps voting and storing logs, but starts no election on its own.
let roles = BTreeMap::from([(3, NodeRole::ReceiveLogs)]);
raft.change_membership(WeightedChange::default().with_roles(roles), false).await?;

// The leader can still hand leadership to voter 3.
raft.trigger().transfer_leader(3).await?;
```

Every write voter needs `ReceiveLogs`, because a write quorum that holds a
voter without logs never commits. `with_roles()` and the change types reject
such a role with [`WriteVoterWithoutReceiveLogs`]. In the weighted,
hierarchical and grid memberships every voter is a write voter. An explicit
membership may list a voter only in its read quorums, and that voter may drop
`ReceiveLogs`, because no commit waits for it.

A change keeps a role only while it differs from the default. So a voter with
only `ReceiveLogs` that becomes a learner has the default role of a learner,
and it campaigns again once a later change makes it a voter.

[`ThresholdMembership`]: crate::ThresholdMembership
[`WeightedMembership`]: crate::WeightedMembership
[`HierarchicalMembership`]: crate::HierarchicalMembership
[`GridMembership`]: crate::GridMembership
[`ExplicitMembership`]: crate::ExplicitMembership
[`WeightedMembership::with_roles()`]: crate::WeightedMembership::with_roles
[`threshold`]: crate::docs::threshold
[`weighted`]: crate::docs::weighted
[`hierarchical`]: crate::docs::hierarchical
[`grid`]: crate::docs::grid
[`explicit`]: crate::docs::explicit
[`NodeRole`]: openraft::membership::NodeRole
[`NodeRole::Campaigner`]: openraft::membership::NodeRole::Campaigner
[`NodeRole::ReceiveLogs`]: openraft::membership::NodeRole::ReceiveLogs
[`Trigger::transfer_leader()`]: openraft::raft::trigger::Trigger::transfer_leader
[`WriteVoterWithoutReceiveLogs`]: openraft::membership::errors::WriteVoterWithoutReceiveLogs
