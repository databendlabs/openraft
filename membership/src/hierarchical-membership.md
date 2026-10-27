# Hierarchical membership: require enough voters in enough groups

[`HierarchicalMembership`] places each voter of a config in one named group,
such as a zone. It sets `write_groups`, and for every group a `write_size`:

- A write quorum holds `write_size` voters in each of `write_groups` groups.
  An entry commits once a write quorum stores it.
- A read quorum holds `read_size = voter_count - write_size + 1` voters of a
  group, in each of `read_groups = group_count - write_groups + 1` groups. A
  candidate becomes leader only with votes from a read quorum.

Thresholds at both levels let a cluster survive the loss of a whole group. For
example, take three zones of three voters, `write_groups = 2`, and
`write_size = 2` in every zone. A read and a write then each need 2 voters in
each of 2 zones. The cluster keeps committing and electing after it loses any
one zone, or one voter in every zone:

```rust
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use openraft::membership::{QuorumSet, RaftMembership};
use openraft_membership::HierarchicalMembership;

let groups = BTreeMap::from([
    ("a".to_string(), BTreeSet::from([1, 2, 3])),
    ("b".to_string(), BTreeSet::from([4, 5, 6])),
    ("c".to_string(), BTreeSet::from([7, 8, 9])),
]);
let write_sizes = BTreeMap::from([("a".to_string(), 2), ("b".to_string(), 2), ("c".to_string(), 2)]);
let nodes = BTreeSet::from([1, 2, 3, 4, 5, 6, 7, 8, 9]);
let membership = HierarchicalMembership::<u64, ()>::new(groups, 2, write_sizes, nodes).unwrap();

// The derived read rule also needs 2 voters in each of 2 groups.
let want = "{configs: [Config { read: HierarchicalQuorumSet { groups: {\"a\": Group { voters: {1, 2, 3}, \
            quorum_size: 2 }, \"b\": Group { voters: {4, 5, 6}, quorum_size: 2 }, \"c\": Group { voters: {7, 8, 9}, \
            quorum_size: 2 }}, quorum_groups: 2 }, write: HierarchicalQuorumSet { groups: {\"a\": Group { \
            voters: {1, 2, 3}, quorum_size: 2 }, \"b\": Group { voters: {4, 5, 6}, quorum_size: 2 }, \"c\": Group { \
            voters: {7, 8, 9}, quorum_size: 2 }}, quorum_groups: 2 }, \
            nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), 2: ((), NodeRole(Campaigner | ReceiveLogs)), \
            3: ((), NodeRole(Campaigner | ReceiveLogs)), 4: ((), NodeRole(Campaigner | ReceiveLogs)), \
            5: ((), NodeRole(Campaigner | ReceiveLogs)), 6: ((), NodeRole(Campaigner | ReceiveLogs)), \
            7: ((), NodeRole(Campaigner | ReceiveLogs)), 8: ((), NodeRole(Campaigner | ReceiveLogs)), \
            9: ((), NodeRole(Campaigner | ReceiveLogs))} }]}";
let got = membership.to_string();
assert_eq!(want, got);

// Zone c is lost: zones a and b still commit and elect.
let live = [1, 2, 3, 4, 5, 6];
let commits = membership.write_quorum_set().is_quorum(live.iter());
assert!(commits);
let elects = membership.read_quorum_set().is_quorum(live.iter());
assert!(elects);
```

An application selects the type in its type config, and passes the membership
to [`Raft::initialize()`], as [`weighted`] shows. The type config sets
`Membership = HierarchicalMembership<..>` and `MembershipEdit = HierarchicalChange<..>`.

## A read quorum and a write quorum share a group, and a voter in it

`read_groups + write_groups` is one more than the group count, so a read
quorum and a write quorum both reach some group. In that group,
`read_size + write_size` is one more than the voter count of the group, so
they share a voter there. The derived read quorums are exactly the sets that
meet every write quorum, so an election needs no more votes than safety
requires.

Because every read quorum meets every write quorum, the new leader holds every
committed entry, as [`weighted`][weighted-intersection] explains.

[`HierarchicalMembership::new()`] and edits return [`InvalidGroups`] inside
[`MembershipError::UserDefined`] when the layout holds no group, when a group
holds no voter, or when two groups share a voter. They also return it unless
`1 <= write_groups <= group_count`, and unless every group has a write size
with `1 <= write_size <= voter_count`.

```rust
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use openraft::errors::MembershipError;
use openraft_membership::HierarchicalMembership;
use openraft_membership::errors::InvalidGroups;

let groups = BTreeMap::from([
    ("a".to_string(), BTreeSet::from([1, 2])),
    ("b".to_string(), BTreeSet::from([3, 4])),
]);
let write_sizes = BTreeMap::from([("a".to_string(), 2), ("b".to_string(), 2)]);
let nodes = BTreeSet::from([1, 2, 3, 4]);

// A write cannot reach 3 of 2 groups.
let res = HierarchicalMembership::<u64, ()>::new(groups, 3, write_sizes, nodes);
let invalid = InvalidGroups::InvalidQuorumGroups { quorum_groups: 3, group_count: 2 };
assert_eq!(Err(MembershipError::UserDefined(invalid)), res);
```

## A commit waits for `write_size` voters in `write_groups` groups, and an election for the read thresholds or more

A commit waits until a write quorum stores the entry, the leader included.
How many votes an election needs depends on the leader ID type:

- With advanced leader IDs, [`leader_id_adv::LeaderId`], the default, an
  election needs `read_size` votes in each of `read_groups` groups.
- With standard leader IDs, [`leader_id_std::LeaderId`], the votes must form
  both a read quorum and a write quorum, because standard leader IDs allow
  only one leader per term. [`election_quorum`] explains why.

The new leader also needs a live write quorum, as
[`weighted`][weighted-election] explains.

For three zones of three voters, with the same `write_size` in every zone:

| `write_groups` | `write_size` | A commit waits for          | Election votes, advanced IDs | Election votes, standard IDs                          |
|----------------|--------------|-----------------------------|------------------------------|-------------------------------------------------------|
| 2              | 2            | 2 voters in each of 2 zones | 2 voters in each of 2 zones  | 2 voters in each of 2 zones                           |
| 1              | 2            | 2 voters in 1 zone          | 2 voters in each of 3 zones  | 2 voters in each of 3 zones                           |
| 3              | 1            | 1 voter in each of 3 zones  | the 3 voters of 1 zone       | the 3 voters of 1 zone and 1 voter in each other zone |

## `write_groups` decides whether losing a whole group stops commits or elections

Writes commit while `write_groups` groups each keep `write_size` live voters.
Elections succeed while `read_groups` groups each keep `read_size` live
voters. For the three zones in the table above:

- With `write_groups = 2` and `write_size = 2`, the cluster survives the loss
  of any one zone, or of one voter in every zone. Losing one zone and 2 voters
  of another stops both commits and elections.
- With `write_groups = 1`, a commit can complete inside the zone of the leader,
  without waiting for another zone. But every election needs 2 voters in every
  zone. After one zone is lost, the leader keeps committing, but no new leader
  can be elected.
- With `write_groups = 3`, every committed entry reaches every zone. Losing one
  zone stops every commit, even though a complete zone still elects a leader
  with advanced leader IDs.

Converting a node map into a `HierarchicalMembership` puts every node in one
group named `default`, with `write_groups = 1` and `write_size = n / 2 + 1`
for `n` voters. That gives the quorums of a [`ThresholdMembership`] converted
from the same node map.

## Change the groups and the thresholds with `HierarchicalChange`

With `MembershipEdit = HierarchicalChange<..>`, [`Raft::change_membership()`]
takes a [`HierarchicalChange`]. [`HierarchicalChange::groups()`] sets the
layout, [`HierarchicalChange::write_groups()`] sets `write_groups`, and
[`HierarchicalChange::write_sizes()`] sets the write sizes of the groups that
it names. A change keeps what it does not set: a group that keeps its ID keeps
its write size, and a group with a new ID needs a write size in the same
change. The change derives the read thresholds from its result. A new voter
must already be a node: add it as a learner first, or with the `add_nodes`
field of the change.

```rust
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use openraft::membership::{RaftMembership, RaftMembershipEdit};
use openraft_membership::{HierarchicalChange, HierarchicalMembership};

let groups = BTreeMap::from([
    ("a".to_string(), BTreeSet::from([1, 2, 3])),
    ("b".to_string(), BTreeSet::from([4, 5, 6])),
    ("c".to_string(), BTreeSet::from([7, 8, 9])),
]);
let write_sizes = BTreeMap::from([("a".to_string(), 2), ("b".to_string(), 2), ("c".to_string(), 2)]);
// Nodes 10, 11 and 12 are learners.
let nodes = BTreeSet::from([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
let membership = HierarchicalMembership::<u64, ()>::new(groups.clone(), 2, write_sizes, nodes).unwrap();

// The learners form zone d with write size 2. A write still reaches 2 zones,
// so a read now reaches 3 of the 4 zones.
let mut four_zones = groups;
four_zones.insert("d".to_string(), BTreeSet::from([10, 11, 12]));
let d_write_size = BTreeMap::from([("d".to_string(), 2)]);
let change = HierarchicalChange::groups(four_zones).with_write_sizes(d_write_size);
let target = change.apply(&membership).unwrap();
let want = "{configs: [Config { read: HierarchicalQuorumSet { groups: {\"a\": Group { voters: {1, 2, 3}, \
            quorum_size: 2 }, \"b\": Group { voters: {4, 5, 6}, quorum_size: 2 }, \"c\": Group { voters: {7, 8, 9}, \
            quorum_size: 2 }, \"d\": Group { voters: {10, 11, 12}, quorum_size: 2 }}, quorum_groups: 3 }, \
            write: HierarchicalQuorumSet { groups: {\"a\": Group { voters: {1, 2, 3}, quorum_size: 2 }, \
            \"b\": Group { voters: {4, 5, 6}, quorum_size: 2 }, \"c\": Group { voters: {7, 8, 9}, quorum_size: 2 }, \
            \"d\": Group { voters: {10, 11, 12}, quorum_size: 2 }}, quorum_groups: 2 }, \
            nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), 2: ((), NodeRole(Campaigner | ReceiveLogs)), \
            3: ((), NodeRole(Campaigner | ReceiveLogs)), 4: ((), NodeRole(Campaigner | ReceiveLogs)), \
            5: ((), NodeRole(Campaigner | ReceiveLogs)), 6: ((), NodeRole(Campaigner | ReceiveLogs)), \
            7: ((), NodeRole(Campaigner | ReceiveLogs)), 8: ((), NodeRole(Campaigner | ReceiveLogs)), \
            9: ((), NodeRole(Campaigner | ReceiveLogs)), 10: ((), NodeRole(Campaigner | ReceiveLogs)), \
            11: ((), NodeRole(Campaigner | ReceiveLogs)), 12: ((), NodeRole(Campaigner | ReceiveLogs))} }]}";
let got = target.to_string();
assert_eq!(want, got);

// The layout changed, so OpenRaft first appends a joint config.
let direct = membership.supports_direct_transition_to(&target);
assert!(!direct);
```

## Changing the groups or the write thresholds requires a joint configuration

OpenRaft appends a changed config directly only when every read quorum of each
config intersects every write quorum of the other. Otherwise it first appends
a joint config, as [`weighted`][weighted-joint] explains.

The hierarchical membership proves this only when both memberships hold a
config with the same groups and the same write thresholds. A read quorum of one
membership and a write quorum of the other then meet the read thresholds and
the write thresholds of that config, so they share a voter. Within one layout
this rule loses no safe change: each read rule is derived from its write rule,
so both directions intersect only when the write thresholds stay the same. So a
direct change keeps the groups and the write thresholds: changing only roles,
adding a learner, and leaving a joint config go directly. Moving a voter to
another group, adding or removing a voter or a group, and changing
`write_groups` or a write size all go through a joint config.

## The hierarchical types connect like the weighted types, with a read rule derived from the write rule

[`HierarchicalMembership`] implements [`RaftMembership`], and
[`HierarchicalChange`] implements [`RaftMembershipEdit`], in the same way as
the weighted types and with the same crate-private `Config`. [`weighted`]
walks through how OpenRaft calls them during a change. It also lists what to
copy to write a membership of your own, where `src/hierarchical/` takes the
place of `src/weighted/`. The hierarchical membership differs in its rule and
in how a change builds it:

- [`HierarchicalQuorumSet`] holds the groups, each a [`Group`] of voters with a
  quorum size, and the number of groups `quorum_groups` that a quorum must
  reach. Its [`QuorumSet`] impl accepts a set that holds the quorum size of
  voters in each of `quorum_groups` groups.
- Each config holds a write rule with `write_groups` and the write sizes, and
  a read rule derived from it, with `read_groups` and the read sizes.
- The [`QuorumIntersection`] impl of `HierarchicalQuorumSet` returns
  `Some(true)` when both rules hold the same groups, their `quorum_groups` sum
  to more than the group count, and the two quorum sizes of each group sum to
  more than its voter count. It returns `None` for every other pair. The read
  rule and the write rule of a config exceed both counts by exactly 1, so
  [`read_write_quorums_intersect()`] returns `Some(true)` for the target of
  every change.
- [`HierarchicalChange`] builds a new write rule from the write rule of the
  last config and the fields that it sets. It then derives the read rule from
  the new write rule.

[`HierarchicalMembership`]: crate::HierarchicalMembership
[`HierarchicalMembership::new()`]: crate::HierarchicalMembership::new
[`HierarchicalChange`]: crate::HierarchicalChange
[`HierarchicalChange::groups()`]: crate::HierarchicalChange::groups
[`HierarchicalChange::write_groups()`]: crate::HierarchicalChange::write_groups
[`HierarchicalChange::write_sizes()`]: crate::HierarchicalChange::write_sizes
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
