# Provided memberships: pick how each config states its quorums

Every membership in this crate gives each config read quorums and write
quorums. An entry commits once a write quorum stores it, and a candidate
becomes leader only with votes from a read quorum. Every read quorum shares a
voter with every write quorum, so a new leader holds every committed entry.
The memberships differ in how a config states its quorums. That decides how
many voters a commit and an election wait for, and which failures stop the
cluster. The first row is OpenRaft's default [`Membership`], which a type config
uses unless it names another one:

| Membership                 | Edit                   | A config sets                                                                     | Pick it when                                                                                             |
|----------------------------|------------------------|-----------------------------------------------------------------------------------|----------------------------------------------------------------------------------------------------------|
| [`Membership`]             | [`ChangeMembers`]      | the voters; a majority of them elects and commits                                 | a majority suits both elections and commits                                                              |
| [`ThresholdMembership`]    | [`ThresholdChange`]    | the voters and a write size                                                       | every voter counts the same, and you choose how many copies a commit needs                               |
| [`WeightedMembership`]     | [`WeightedChange`]     | a weight for each voter and a write weight                                        | some voters, or the sites that hold them, must count more                                                |
| [`HierarchicalMembership`] | [`HierarchicalChange`] | a tree of voter groups, such as zones, and how many members of each a write needs | the cluster must keep running after it loses a whole zone                                                |
| [`GridMembership`]         | [`GridChange`]         | a rectangular grid of voters                                                      | a commit must wait for fewer voters than a majority, and so must an election with the default leader IDs |
| [`ExplicitMembership`]     | [`ExplicitChange`]     | the minimal read quorums and the minimal write quorums                            | no rule above states the policy, such as a backup voter in every write quorum and in no read quorum      |

Each row names the type for `RaftTypeConfig::Membership` and the edit type for
`RaftTypeConfig::MembershipEdit`, which [`Raft::change_membership()`] takes.

The guides [`threshold`], [`weighted`], [`hierarchical`], [`grid`] and
[`explicit`] each show how to configure the membership, why its read and
write quorums intersect, what a commit and an election wait for, which
failures stop it, and how to change it. [`weighted`] also walks through how
its quorum rule, its membership and its change implement the traits that
OpenRaft calls, and what to copy to write a membership of your own. The
hierarchical, grid and explicit guides name what their types do differently.

## Use a provided membership in an application

The [`raft-kv-memstore-weighted-membership`] example is the HTTP key-value
store of `raft-kv-memstore` with a [`WeightedMembership`]. Its type config
names `WeightedMembership` and `WeightedChange`, and its `/init` and
`/change-membership` routes build them from the weights in a JSON body: the
membership routes that the other examples share take only a set of voter IDs.
Its test runs four node processes, gives the voters the weights
`{1:2, 2:1, 3:1, 4:1}` with write weight 3, and stops voters 3 and 4. Voters 1
and 2 weigh 3 together, so they keep committing.

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

These four change types share a [`NodeChange`] in their `node_change` field.
Each of the four memberships sets roles with its `with_roles()` method, such
as [`WeightedMembership::with_roles()`]. Each change type sets them with its
`node_change.roles` field or its `with_roles()` builder. A change that sets
only roles keeps every quorum, so OpenRaft appends it without a joint config:

```rust,no_run
# use std::collections::BTreeMap;
#
# use openraft::BasicNode;
# use openraft::Raft;
# use openraft::membership::NodeRole;
# use openraft::storage::RaftStateMachine;
# use openraft_membership::WeightedChange;
#
# openraft::declare_raft_types!(
#     pub TypeConfig:
#         D = String,
#         R = String,
#         Membership = openraft_membership::WeightedMembership<u64, BasicNode>,
#         MembershipEdit = openraft_membership::WeightedChange<Self::NodeId, Self::Node>,
#         AsyncRuntime = openraft_rt_tokio::TokioRuntime,
# );
#
# async fn set_roles<SM>(raft: Raft<TypeConfig, SM>) -> Result<(), Box<dyn std::error::Error>>
# where SM: RaftStateMachine<TypeConfig> {
// Voter 3 keeps voting and storing logs, but starts no election on its own.
let roles = BTreeMap::from([(3, NodeRole::ReceiveLogs)]);
raft.change_membership(WeightedChange::default().with_roles(roles), false).await?;

// The leader can still hand leadership to voter 3.
raft.trigger().transfer_leader(3).await?;
# Ok(())
# }
```

Every write voter needs `ReceiveLogs`, because a write quorum that holds a
voter without logs never commits. `with_roles()` and the change types keep a
role as given; OpenRaft rejects a membership whose write voter lacks
`ReceiveLogs` with [`WriteVoterWithoutReceiveLogs`] when the membership is
initialized or proposed. In the weighted, hierarchical and grid memberships
every voter is a write voter. An explicit membership may list a voter only in
its read quorums, and that voter may drop `ReceiveLogs`, because no commit
waits for it.

A role that `with_roles()` or a change sets stays until a later change sets
another one or removes the node. So a voter with only `ReceiveLogs` that
becomes a learner, and later a voter again, still does not campaign. Setting a
node to the default role for its status clears its set role, and the node
follows the default again.

Each config stores its nodes, each with the role set for it, if any, and
computes the default role of a node without one from its status in that
config. During a joint change, a node's role combines its roles in the new
config and every earlier config where it is a voter. Finalization appends the
computed target, restoring that config's roles.

[`Membership`]: openraft::impls::Membership
[`ChangeMembers`]: openraft::impls::ChangeMembers
[`ThresholdMembership`]: crate::ThresholdMembership
[`ThresholdChange`]: crate::ThresholdChange
[`WeightedChange`]: crate::WeightedChange
[`HierarchicalChange`]: crate::HierarchicalChange
[`GridChange`]: crate::GridChange
[`ExplicitChange`]: crate::ExplicitChange
[`Raft::change_membership()`]: openraft::Raft::change_membership
[`raft-kv-memstore-weighted-membership`]: https://github.com/databendlabs/openraft/tree/main/examples/raft-kv-memstore-weighted-membership
[`WeightedMembership`]: crate::WeightedMembership
[`HierarchicalMembership`]: crate::HierarchicalMembership
[`GridMembership`]: crate::GridMembership
[`ExplicitMembership`]: crate::ExplicitMembership
[`NodeChange`]: crate::NodeChange
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
