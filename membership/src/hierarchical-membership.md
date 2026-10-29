# Hierarchical membership: require enough members in enough groups, at every level

[`HierarchicalMembership`] arranges the voters of a config in a tree of
[`Group`]s, such as datacenters that hold racks. A group holds voters, child
groups, or both: its members. A set reaches a group when the voters of the
group that it holds, plus the child groups that it reaches, number at least
the size of the group. The config gives every group a `write_size`:

- A write quorum reaches the root group with every group at its `write_size`.
  An entry commits once a write quorum stores it.
- A read quorum reaches the root group with every group at its
  `read_size = member_count - write_size + 1`. A candidate becomes leader only
  with votes from a read quorum.

Sizes at every level let a cluster survive the loss of a whole group. For
example, take three zones of three voters, `write_size = 2` at the root, and
`write_size = 2` in every zone. A read and a write then each need 2 voters in
each of 2 zones. The cluster keeps committing and electing after it loses any
one zone, or one voter in every zone:

```rust
use std::collections::BTreeSet;

use openraft::membership::{QuorumSet, RaftMembership};
use openraft_membership::HierarchicalMembership;
use openraft_membership::quorum::Group;

let groups = Group::of_groups(2, [
    ("a", Group::of_voters(2, [1, 2, 3])),
    ("b", Group::of_voters(2, [4, 5, 6])),
    ("c", Group::of_voters(2, [7, 8, 9])),
]);
let nodes = BTreeSet::from([1, 2, 3, 4, 5, 6, 7, 8, 9]);
let membership = HierarchicalMembership::<u64, ()>::new(groups.clone(), nodes).unwrap();

// The derived read sizes are 3 - 2 + 1 = 2 at every level, so a read also
// needs 2 voters in each of 2 zones.
let read = membership.read_quorum_set();
let read_rule = &read.rules()[0];
assert_eq!(&groups, read_rule.root());

// Zone c is lost: zones a and b still commit and elect.
let live = [1, 2, 3, 4, 5, 6];
let commits = membership.write_quorum_set().is_quorum(live.iter());
assert!(commits);
let elects = membership.read_quorum_set().is_quorum(live.iter());
assert!(elects);
```

Groups nest to any depth, and the branches of the tree may differ in depth.
For example, datacenter `dc1` holds racks `r1 = {1, 2}` and `r2 = {3, 4}`,
datacenter `dc2` holds voters 5 and 6, and the root also holds a witness voter
7. A write reaches 2 of `dc1`, `dc2` and the witness: `dc1` with a voter of
each rack, and `dc2` with both of its voters. The derived read reaches `dc1`
with both voters of one rack, and `dc2` with one voter:

```rust
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use openraft::membership::{QuorumSet, RaftMembership};
use openraft_membership::HierarchicalMembership;
use openraft_membership::quorum::Group;

let dc1 = Group::of_groups(2, [
    ("r1", Group::of_voters(1, [1, 2])),
    ("r2", Group::of_voters(1, [3, 4])),
]);
let groups = Group {
    quorum_size: 2,
    voters: BTreeSet::from([7]),
    groups: BTreeMap::from([
        ("dc1".to_string(), dc1),
        ("dc2".to_string(), Group::of_voters(2, [5, 6])),
    ]),
};
let nodes = BTreeSet::from([1, 2, 3, 4, 5, 6, 7]);
let membership = HierarchicalMembership::<u64, ()>::new(groups, nodes).unwrap();

// dc2 is lost: dc1 and the witness still commit and elect.
let live = [1, 2, 3, 4, 7];
let commits = membership.write_quorum_set().is_quorum(live.iter());
assert!(commits);
let elects = membership.read_quorum_set().is_quorum(live.iter());
assert!(elects);

// Rack r2 and voter 6 are lost: no write reaches dc1 or dc2, but a read still
// reaches both.
let live = [1, 2, 5, 7];
let commits = membership.write_quorum_set().is_quorum(live.iter());
assert!(!commits);
let elects = membership.read_quorum_set().is_quorum(live.iter());
assert!(elects);
```

An application selects the type in its type config, and passes the membership
to [`Raft::initialize()`], as [`weighted`] shows. The type config sets
`Membership = HierarchicalMembership<..>` and `MembershipEdit = HierarchicalChange<..>`.

## A read quorum and a write quorum share a member in every group that both reach, down to a voter

In the root group, `read_size + write_size` is one more than the member count,
so a read quorum and a write quorum both reach some member of it. A common
voter is a shared voter. A common child group has the same property, so the
two quorums again reach a common member there, and so on down the tree until
they share a voter. The derived read quorums are exactly the sets that meet
every write quorum, so an election needs no more votes than safety requires.

Because every read quorum meets every write quorum, the new leader holds every
committed entry, as [`weighted`][weighted-intersection] explains.

[`HierarchicalMembership::new()`] and edits return [`InvalidGroups`] inside
[`MembershipError::UserDefined`] when a group holds no member, when two groups
hold the same voter, or unless every group has
`1 <= write_size <= member_count`. The error names a group by its path, the IDs
of the groups from the root down to it:

```rust
use std::collections::BTreeSet;

use openraft::errors::MembershipError;
use openraft_membership::HierarchicalMembership;
use openraft_membership::errors::InvalidGroups;
use openraft_membership::quorum::Group;

// A write cannot reach 3 of the 2 racks of dc1.
let dc1 = Group::of_groups(3, [
    ("r1", Group::of_voters(1, [1, 2])),
    ("r2", Group::of_voters(1, [3, 4])),
]);
let groups = Group::of_groups(1, [("dc1", dc1)]);
let nodes = BTreeSet::from([1, 2, 3, 4]);
let res = HierarchicalMembership::<u64, ()>::new(groups, nodes);

let invalid = InvalidGroups::InvalidQuorumSize {
    group: vec!["dc1".to_string()],
    quorum_size: 3,
    member_count: 2,
};
assert_eq!(Err(MembershipError::UserDefined(invalid)), res);
```

## A commit waits for the write sizes, and an election for the read sizes or more

A commit waits until a write quorum stores the entry, the leader included.
How many votes an election needs depends on the leader ID type:

- With advanced leader IDs, [`leader_id_adv::LeaderId`], the default, an
  election needs votes that reach the root group with the read sizes.
- With standard leader IDs, [`leader_id_std::LeaderId`], the votes must form
  both a read quorum and a write quorum, because standard leader IDs allow
  only one leader per term. [`election_quorum`] explains why.

The new leader also needs a live write quorum, as
[`weighted`][weighted-election] explains.

For three zones of three voters, with the same `write_size` in every zone:

| Root `write_size` | Zone `write_size` | A commit waits for          | Election votes, advanced IDs | Election votes, standard IDs                          |
|-------------------|-------------------|-----------------------------|------------------------------|-------------------------------------------------------|
| 2                 | 2                 | 2 voters in each of 2 zones | 2 voters in each of 2 zones  | 2 voters in each of 2 zones                           |
| 1                 | 2                 | 2 voters in 1 zone          | 2 voters in each of 3 zones  | 2 voters in each of 3 zones                           |
| 3                 | 1                 | 1 voter in each of 3 zones  | the 3 voters of 1 zone       | the 3 voters of 1 zone and 1 voter in each other zone |

## The root `write_size` decides whether losing a whole group stops commits or elections

Writes commit while the live voters reach the root group with the write sizes.
Elections succeed while they reach it with the read sizes. For the three zones
in the table above:

- With `write_size = 2` at the root and in every zone, the cluster survives the
  loss of any one zone, or of one voter in every zone. Losing one zone and 2
  voters of another stops both commits and elections.
- With `write_size = 1` at the root, a commit can complete inside the zone of
  the leader, without waiting for another zone. But every election needs 2
  voters in every zone. After one zone is lost, the leader keeps committing,
  but no new leader can be elected.
- With `write_size = 3` at the root, every committed entry reaches every zone.
  Losing one zone stops every commit, even though a complete zone still elects
  a leader with advanced leader IDs.

The `write_size` of a group inside the tree makes the same trade within that
group. In the datacenter example above, `dc1` with `write_size = 2` takes no
write after it loses a rack, but a read still reaches it through both voters
of the other rack.

Converting a node map into a `HierarchicalMembership` puts every node in the
root group, with `write_size = n / 2 + 1` for `n` voters. That gives the
quorums of a [`ThresholdMembership`] converted from the same node map.

## Change the groups and the write sizes with `HierarchicalChange`

With `MembershipEdit = HierarchicalChange<..>`, [`Raft::change_membership()`]
takes a [`HierarchicalChange`]. [`HierarchicalChange::groups()`] sets a new
root group, with the write size of every group in its tree. A change keeps the
groups and their write sizes unless it sets a new root group, and it derives
the read sizes from its result. A new voter must already be a node: add it as
a learner first, or with the `add_nodes` field of the change.

```rust
use std::collections::BTreeSet;

use openraft::membership::{QuorumSet, RaftMembership, RaftMembershipEdit};
use openraft_membership::quorum::Group;
use openraft_membership::{HierarchicalChange, HierarchicalMembership};

let groups = Group::of_groups(2, [
    ("a", Group::of_voters(2, [1, 2, 3])),
    ("b", Group::of_voters(2, [4, 5, 6])),
    ("c", Group::of_voters(2, [7, 8, 9])),
]);
// Nodes 10, 11 and 12 are learners.
let nodes = BTreeSet::from([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
let membership = HierarchicalMembership::<u64, ()>::new(groups, nodes).unwrap();

// The learners form zone d with write size 2. A write still reaches 2 zones,
// so a read now reaches 3 of the 4 zones.
let four_zones = Group::of_groups(2, [
    ("a", Group::of_voters(2, [1, 2, 3])),
    ("b", Group::of_voters(2, [4, 5, 6])),
    ("c", Group::of_voters(2, [7, 8, 9])),
    ("d", Group::of_voters(2, [10, 11, 12])),
]);
let change = HierarchicalChange::groups(four_zones);
let target = change.apply(&membership, true).unwrap();

let elects = target.read_quorum_set().is_quorum([1, 2, 4, 5].iter());
assert!(!elects);
let elects = target.read_quorum_set().is_quorum([1, 2, 4, 5, 7, 8].iter());
assert!(elects);

// The groups changed, so OpenRaft first appends a joint config.
let direct = membership.supports_direct_transition_to(&target);
assert!(!direct);
```

## Changing the groups or the write sizes requires a joint configuration

OpenRaft appends a changed config directly only when every read quorum of each
config intersects every write quorum of the other. Otherwise it first appends
a joint config, as [`weighted`][weighted-joint] explains.

The hierarchical membership proves this only when both memberships hold a
config with the same tree and the same write sizes. A read quorum of one
membership and a write quorum of the other then meet the read sizes and the
write sizes of that config, so they share a voter. Within one tree this rule
loses no safe change: each read rule is derived from its write rule, so both
directions intersect only when the write sizes stay the same. So a direct
change keeps the groups and the write sizes: changing only roles, adding a
learner, and leaving a joint config go directly. Moving a voter to another
group, adding or removing a voter or a group at any level, and changing a
write size all go through a joint config.

## The hierarchical types connect like the weighted types, with a read rule derived from the write rule

[`HierarchicalMembership`] implements [`RaftMembership`], and
[`HierarchicalChange`] implements [`RaftMembershipEdit`], in the same way as
the weighted types and with the same crate-private `Config`. [`weighted`]
walks through how OpenRaft calls them during a change. It also lists what to
copy to write a membership of your own, where `src/hierarchical/` takes the
place of `src/weighted/`. The hierarchical membership differs in its rule and
in how a change builds it:

- [`HierarchicalQuorumSet`] holds the root [`Group`]. Each group holds voters,
  child groups, and a quorum size: the number of its members that a quorum
  must reach. Its [`QuorumSet`] impl walks the tree from the root and accepts a
  set that reaches the root group.
- Each config holds a write rule with the write sizes, and a read rule derived
  from it, with the read sizes.
- The [`QuorumIntersection`] impl of `HierarchicalQuorumSet` returns
  `Some(true)` when both rules hold the same tree, and the two quorum sizes of
  each group sum to more than its member count. It returns `None` for every
  other pair. The read rule and the write rule of a config exceed every member
  count by exactly 1, so [`read_write_quorums_intersect()`] returns
  `Some(true)` for the target of every change.
- [`HierarchicalChange`] builds a new write rule from the root group that it
  sets, or keeps the write rule of the last config. It then derives the read
  rule from the write rule.

[`HierarchicalMembership`]: crate::HierarchicalMembership
[`HierarchicalMembership::new()`]: crate::HierarchicalMembership::new
[`HierarchicalChange`]: crate::HierarchicalChange
[`HierarchicalChange::groups()`]: crate::HierarchicalChange::groups
[`ThresholdMembership`]: crate::ThresholdMembership
[`InvalidGroups`]: crate::errors::InvalidGroups
[`MembershipError::UserDefined`]: openraft::errors::MembershipError::UserDefined
[`Raft::initialize()`]: openraft::Raft::initialize
[`Raft::change_membership()`]: openraft::Raft::change_membership
[`leader_id_adv::LeaderId`]: openraft::impls::leader_id_adv::LeaderId
[`leader_id_std::LeaderId`]: openraft::impls::leader_id_std::LeaderId
[`election_quorum`]: openraft::docs::protocol::election_quorum
[`weighted`]: crate::docs::weighted
[weighted-intersection]: crate::docs::weighted#a-read-quorum-and-a-write-quorum-together-weigh-more-than-all-voters
[weighted-election]: crate::docs::weighted#a-commit-waits-for-write_weight-and-an-election-for-the-read-weight-or-more
[weighted-joint]: crate::docs::weighted#every-change-of-the-weights-or-the-write-weight-requires-a-joint-configuration
[`HierarchicalQuorumSet`]: crate::quorum::HierarchicalQuorumSet
[`Group`]: crate::quorum::Group
[`QuorumSet`]: openraft::membership::QuorumSet
[`QuorumIntersection`]: openraft::membership::QuorumIntersection
[`RaftMembership`]: openraft::membership::RaftMembership
[`RaftMembershipEdit`]: openraft::membership::RaftMembershipEdit
[`read_write_quorums_intersect()`]: openraft::membership::RaftMembership::read_write_quorums_intersect
