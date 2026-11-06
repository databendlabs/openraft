# Provided memberships: pick how each config states its quorums

Every membership in this crate gives each config read quorums and write
quorums. An entry commits once a write quorum stores it, and a candidate
becomes leader only with votes from a read quorum. Every read quorum shares a
voter with every write quorum, so a new leader holds every committed entry.
The memberships differ in how a config states its quorums. That decides how
many voters a commit and an election wait for, and which failures stop the
cluster. The first row is OpenRaft's default [`Membership`], which a type config
uses unless it names another one:

| Membership                 | Edit                   | A config sets                                                                     | Pick it when                                                                                             | Runnable scenario   |
|----------------------------|------------------------|-----------------------------------------------------------------------------------|----------------------------------------------------------------------------------------------------------|---------------------|
| [`Membership`]             | [`ChangeMembers`]      | the voters; a majority of them elects and commits                                 | a majority suits both elections and commits                                                              | [`majority.rs`]     |
| [`ThresholdMembership`]    | [`ThresholdChange`]    | the voters and a write size                                                       | every voter counts the same, and you choose how many copies a commit needs                               | [`threshold.rs`]    |
| [`WeightedMembership`]     | [`WeightedChange`]     | a weight for each voter and a write weight                                        | some voters, or the sites that hold them, must count more                                                | [`weighted.rs`]     |
| [`HierarchicalMembership`] | [`HierarchicalChange`] | a tree of voter groups, such as zones, and how many members of each a write needs | the cluster must keep running after it loses a whole zone                                                | [`hierarchical.rs`] |
| [`GridMembership`]         | [`GridChange`]         | a rectangular grid of voters                                                      | a commit must wait for fewer voters than a majority, and so must an election with the default leader IDs | [`grid.rs`]         |
| [`ExplicitMembership`]     | [`ExplicitChange`]     | the minimal read quorums and the minimal write quorums                            | no rule above states the policy, such as a backup voter in every write quorum and in no read quorum      | [`explicit.rs`]     |

Each row names the type for `RaftTypeConfig::Membership` and the edit type for
`RaftTypeConfig::MembershipEdit`, which [`Raft::change_membership()`] takes. Its
scenario is a complete program that declares both types and runs them.

The guides [`threshold`], [`weighted`], [`hierarchical`], [`grid`] and
[`explicit`] each show how to configure the membership, why its read and
write quorums intersect, what a commit and an election wait for, which
failures stop it, and how to change it. [`weighted`] also walks through how
its quorum rule, its membership and its change implement the traits that
OpenRaft calls, and what to copy to write a membership of your own. The
hierarchical, grid and explicit guides name what their types do differently.

## Run each membership in an in-process cluster

The [`examples/membership`] application runs every scenario in a cluster of
`Raft` nodes that start from empty stores:

```shell
cargo run --manifest-path examples/membership/Cargo.toml -- --policy weighted
```

`--policy` takes `majority`, `threshold`, `weighted`, `hierarchical`, `grid` or
`explicit`. A run initializes node 1 with an explicit membership through
[`Raft::initialize()`], writes a value, changes the membership with the edit
type, writes again, isolates some nodes, and heals them. It checks every
response and every replicated value, logs each phase, and ends with
`every check passed`. Node 1 leads throughout. During the isolation, each
scenario checks the available nodes against both quorum rules:

| `--policy`     | Edit                                                                                               | Available during isolation    | Elects | Commits | Result                                                        |
|----------------|----------------------------------------------------------------------------------------------------|-------------------------------|--------|---------|---------------------------------------------------------------|
| `majority`     | add learner 4 to voters `{1,2,3}`, then `ChangeMembers::AddVoterIds({4})`                          | `{1,2,3}`                     | yes    | yes     | writes continue                                               |
| `threshold`    | `ThresholdChange::write_size(2)` on 5 voters with write size 3                                     | `{1,2}`                       | no     | yes     | writes continue, but no election could succeed                |
| `weighted`     | `WeightedChange::weights()` moves the extra weight of voter 1 to voter 2                           | `{1,2}`                       | yes    | yes     | writes continue                                               |
| `hierarchical` | `HierarchicalChange::groups()` lowers the root write size of 3 zones from 3 to 2                   | zones `{1,2,3}` and `{4,5,6}` | yes    | yes     | writes continue without the third zone                        |
| `grid`         | `GridChange::grid()` transposes a 3 x 3 grid                                                       | `{1,5,9}`                     | no     | yes     | writes continue, but no election could succeed                |
| `explicit`     | `ExplicitChange::read_quorums()` narrows reads to `{1,2}`; every write quorum keeps backup voter 9 | `{1,2,3}`                     | yes    | no      | a write stays pending until voter 9 is healed, then completes |

"Elects" tells whether the available nodes hold a read quorum of the changed
membership, and "Commits" whether they hold a write quorum. The leader keeps
leading, so a missing read quorum only means that no new leader could be
elected. A newly elected leader also needs a write quorum: it completes no
write and no linearizable read before it commits the blank entry of its term.

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

```rust,ignore
// Voter 3 keeps voting and storing logs, but starts no election on its own.
let roles = BTreeMap::from([(3, NodeRole::ReceiveLogs)]);
raft.change_membership(WeightedChange::default().with_roles(roles), false).await?;

// The leader can still hand leadership to voter 3.
raft.trigger().transfer_leader(3).await?;
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
config and every earlier config where it is a voter. [`make_uniform()`] keeps
only the new config, restoring that config's roles.

[`Membership`]: openraft::impls::Membership
[`ChangeMembers`]: openraft::impls::ChangeMembers
[`ThresholdMembership`]: crate::ThresholdMembership
[`ThresholdChange`]: crate::ThresholdChange
[`WeightedChange`]: crate::WeightedChange
[`HierarchicalChange`]: crate::HierarchicalChange
[`GridChange`]: crate::GridChange
[`ExplicitChange`]: crate::ExplicitChange
[`Raft::initialize()`]: openraft::Raft::initialize
[`Raft::change_membership()`]: openraft::Raft::change_membership
[`examples/membership`]: https://github.com/databendlabs/openraft/tree/main/examples/membership
[`majority.rs`]: https://github.com/databendlabs/openraft/blob/main/examples/membership/src/scenarios/majority.rs
[`threshold.rs`]: https://github.com/databendlabs/openraft/blob/main/examples/membership/src/scenarios/threshold.rs
[`weighted.rs`]: https://github.com/databendlabs/openraft/blob/main/examples/membership/src/scenarios/weighted.rs
[`hierarchical.rs`]: https://github.com/databendlabs/openraft/blob/main/examples/membership/src/scenarios/hierarchical.rs
[`grid.rs`]: https://github.com/databendlabs/openraft/blob/main/examples/membership/src/scenarios/grid.rs
[`explicit.rs`]: https://github.com/databendlabs/openraft/blob/main/examples/membership/src/scenarios/explicit.rs
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
[`make_uniform()`]: openraft::membership::RaftMembershipEdit::make_uniform
