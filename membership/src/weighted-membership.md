# Weighted membership: let some voters count more

[`WeightedMembership`] gives each voter of a config a positive weight and sets
one `write_weight`. Let `total_weight` be the sum of the weights:

- A set of voters is a write quorum when its weights sum to at least
  `write_weight`. An entry commits once a write quorum stores it.
- A set of voters is a read quorum when its weights sum to at least
  `read_weight = total_weight - write_weight + 1`, the minimum that
  intersects every write quorum. A candidate becomes leader only with votes
  from a read quorum.

With weight 1 for every voter, `write_weight` is the `write_size` of a
[`ThresholdMembership`], which [`threshold`] describes. Unequal weights let a
voter, or the site that holds it, count more.

For example, put four voters in two sites, `{1,2}` and `{3,4}`. With equal
weights, a site of 2 voters never holds both a read quorum and a write quorum,
so losing either site stops commits or elections. Weight 2 on voter 1 makes
site `{1,2}` weigh 3 of the total 5. With `write_weight = 3`, the read weight
is also 3, so that site commits and elects on its own:

```rust
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use openraft::membership::{QuorumSet, RaftMembership};
use openraft_membership::WeightedMembership;

// The total weight is 5, so write weight 3 derives read weight 3.
let weights = BTreeMap::from([(1, 2), (2, 1), (3, 1), (4, 1)]);
let nodes = BTreeSet::from([1, 2, 3, 4]);
let membership = WeightedMembership::<u64, ()>::new(weights, 3, nodes).unwrap();

// Site {1,2} weighs 3: it commits and elects without site {3,4}.
let commits = membership.write_quorum_set().is_quorum([1, 2].iter());
assert!(commits);
let elects = membership.read_quorum_set().is_quorum([1, 2].iter());
assert!(elects);

// Site {3,4} weighs 2: it does neither without site {1,2}.
let commits = membership.write_quorum_set().is_quorum([3, 4].iter());
assert!(!commits);
let elects = membership.read_quorum_set().is_quorum([3, 4].iter());
assert!(!elects);
```

An application selects the type in its type config, and passes the membership
to [`Raft::initialize()`]:

```rust,ignore
openraft::declare_raft_types!(
    pub TypeConfig:
        D = Request,
        R = Response,
        Membership = openraft_membership::WeightedMembership<u64, BasicNode>,
        MembershipEdit = openraft_membership::WeightedChange<Self::NodeId, Self::Node>,
);

raft.initialize(membership).await?;
```

## A read quorum and a write quorum together weigh more than all voters

A read quorum weighs at least `total_weight - write_weight + 1`, and a write
quorum weighs at least `write_weight`. So together they weigh at least
`total_weight + 1`. Two sets that share no voter weigh at most `total_weight`
together, so every read quorum shares a voter with every write quorum. A
smaller read weight could fit outside a write quorum: read weight 2 in the
example above would accept site `{3,4}`, which misses the write quorum
`{1,2}`.

Every election thus reaches a voter that stores each committed entry. That
voter votes only for a candidate whose log is at least as up to date as its
own, so the new leader holds every committed entry.

[`WeightedMembership::new()`] and edits return [`InvalidWeights`] inside
[`MembershipError::UserDefined`] for a weight of 0, for weights that sum to
more than `u64::MAX`, and unless `1 <= write_weight <= total_weight`.

```rust
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use openraft::errors::MembershipError;
use openraft_membership::WeightedMembership;
use openraft_membership::errors::InvalidWeights;

// The weights sum to 5, so no set reaches write weight 6.
let weights = BTreeMap::from([(1, 2), (2, 1), (3, 1), (4, 1)]);
let nodes = BTreeSet::from([1, 2, 3, 4]);
let res = WeightedMembership::<u64, ()>::new(weights, 6, nodes);

let invalid = InvalidWeights::WriteWeightOutOfRange { write_weight: 6, total_weight: 5 };
assert_eq!(Err(MembershipError::UserDefined(invalid)), res);
```

## A commit waits for `write_weight`, and an election for the read weight or more

A commit waits until the voters that store the entry, the leader included,
weigh `write_weight` together. So the leader waits only for its fastest
followers whose weight, with its own, reaches `write_weight`.

How much vote weight an election needs depends on the leader ID type:

- With advanced leader IDs, [`leader_id_adv::LeaderId`], the default, an
  election needs votes that weigh `read_weight`.
- With standard leader IDs, [`leader_id_std::LeaderId`], an election needs
  votes that weigh `max(read_weight, write_weight)`. Standard leader IDs allow
  only one leader per term, so the votes must form both a read quorum and a
  write quorum. [`election_quorum`] explains why.

The new leader also needs a live write quorum. It completes a write or a
linearizable read only after it commits the blank entry of its term.

In the example above, the read weight and the write weight are both 3. So a
commit and an election each need voter 1 and one more voter, or voters 2, 3
and 4 together, with either leader ID type.

## A voter that alone weighs `write_weight` commits alone, and every election needs it

The weights decide which failures stop the cluster. A voter that alone weighs
`write_weight` commits an entry by itself, with one copy. Every read quorum
meets that one-voter write quorum, so every read quorum holds the voter, and
no election succeeds while it is down. In the same way, a voter that alone
weighs `read_weight` elects a leader by itself with advanced leader IDs, and
every commit then needs it.

For the example above:

- Write weight 2 lets voter 1 commit alone, and no new leader is elected while
  voter 1 is down.
- Write weight 4 lets voter 1 alone elect a leader, and no entry commits while
  voter 1 is down.
- Write weight 3 makes no single voter necessary: after voter 1 fails, voters
  2, 3 and 4 still weigh 3. Any second failure then stops the cluster.

Converting a node map into a `WeightedMembership` gives every voter weight 1
and `write_weight = n / 2 + 1` for `n` voters. That gives the quorums of a
[`ThresholdMembership`] converted from the same node map.

## Change the weights with `WeightedChange`

With `MembershipEdit = WeightedChange<..>`, [`Raft::change_membership()`]
takes a [`WeightedChange`]. [`WeightedChange::weights()`] sets the voters and
their weights, and [`WeightedChange::write_weight()`] sets the write weight. A
change keeps what it does not set, and derives the read weight from the new
total weight. When the new weights sum to less than the write weight, the same
change must also set a smaller write weight. A new voter must already be a
node: add it as a learner first, or with the `add_nodes` field of the change.

```rust
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use openraft::membership::{RaftMembership, RaftMembershipEdit};
use openraft_membership::{WeightedChange, WeightedMembership};

let weights = BTreeMap::from([(1, 2), (2, 1), (3, 1), (4, 1)]);
let nodes = BTreeSet::from([1, 2, 3, 4]);
let membership = WeightedMembership::<u64, ()>::new(weights, 3, nodes).unwrap();

// Move the extra weight from voter 1 to voter 3, so that site {3,4} outlives
// site {1,2}.
let weights = BTreeMap::from([(1, 1), (2, 1), (3, 2), (4, 1)]);
let change = WeightedChange::weights(weights);
let moved = change.apply(&membership, true).unwrap();
let want = "{configs: [Config { read: WeightedQuorumSet { weights: {1: 1, 2: 1, 3: 2, 4: 1}, threshold: 3 }, \
            write: WeightedQuorumSet { weights: {1: 1, 2: 1, 3: 2, 4: 1}, threshold: 3 }, \
            nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), 2: ((), NodeRole(Campaigner | ReceiveLogs)), \
            3: ((), NodeRole(Campaigner | ReceiveLogs)), 4: ((), NodeRole(Campaigner | ReceiveLogs))} }]}";
let got = moved.to_string();
assert_eq!(want, got);

// The old write quorum {1,2} misses the new read quorum {3,4}, so OpenRaft
// first appends a joint config.
let direct = membership.supports_direct_transition_to(&moved);
assert!(!direct);
```

## Every change of the weights or the write weight requires a joint configuration

OpenRaft appends a changed config directly only when every read quorum of each
config intersects every write quorum of the other. Otherwise it first appends
a joint config, whose quorums are quorums of both configs, and then the new
config.

The weighted membership proves this only when both memberships hold a config
with the same weights and the same write weight. A read quorum of one
membership and a write quorum of the other then reach the read weight and the
write weight of that config, so they share a voter. So every change of the
weights or of the write weight goes through a joint config, even one that
would be safe directly. Changing only roles, adding a learner, and leaving a
joint config go directly.

With the same weights, raising the write weight lowers the read weight, so an
old write quorum may miss a new read quorum. Lowering it causes the reverse
problem.

## Three types implement the traits that OpenRaft calls: the rule, the membership and the change

OpenRaft uses a weighted membership through three types:

- [`WeightedQuorumSet`] is one quorum rule: a weight for each voter and a
  threshold. Its [`QuorumSet`] impl decides whether a set of voters is a
  quorum: [`is_quorum()`] sums the weights of the voters in the set, and
  accepts the set when the sum reaches the threshold. Its
  [`QuorumIntersection`] impl proves that the quorums of two rules intersect:
  [`intersects_with()`] returns `Some(true)` when both rules hold the same
  weights and their thresholds sum to more than the total weight. It returns
  `None` for every other pair.
- [`WeightedMembership`] implements [`RaftMembership`]. It holds a list of
  configs: one config, or the old configs and the new one during a joint
  change. Each config holds a read rule with threshold `read_weight` and a
  write rule with threshold `write_weight` over the same weights, and every
  node with its metadata and its role. [`read_quorum_set()`] returns a
  [`Joint`] of the read rules of all configs, so a read quorum must be a read
  quorum of every config. [`write_quorum_set()`] does the same with the write
  rules.
- [`WeightedChange`] implements [`RaftMembershipEdit`]. Its [`apply()`] builds
  a membership with one config from the last config of the current membership
  and the fields of the change. Its `retain` argument decides whether removed
  voters remain as learners.

[`Raft::change_membership()`] calls them in four steps:

1. `apply(current, retain)` turns the current membership into the target.
   With `retain = false`, it drops each old voter that becomes a learner.
2. OpenRaft rejects the target unless [`read_write_quorums_intersect()`]
   returns `Some(true)`. By default that method calls `intersects_with()` on
   the read quorum set and the write quorum set of the target. The target
   holds one config, so `Joint` passes the call to its two weighted rules.
   Their thresholds sum to `total_weight + 1`, so they prove the intersection.
3. When [`supports_direct_transition_to()`] returns `true`, OpenRaft appends
   the target. Otherwise it appends [`joint_with()`]: the configs of the
   current membership followed by the config of the target.
4. Once the joint commits, OpenRaft applies the change from [`make_uniform()`]
   to the joint and appends the result. That change sets nothing, so its
   `apply()` returns the last config of the joint, which is the target.

[`Raft::add_learner()`] applies the change from [`add_learner()`]. That change
adds one node and keeps both rules, so OpenRaft appends its result directly.

```rust
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use openraft::membership::{QuorumSet, RaftMembership, RaftMembershipEdit};
use openraft_membership::{WeightedChange, WeightedMembership};

type Membership = WeightedMembership<u64, ()>;
type Change = WeightedChange<u64, ()>;

let weights = BTreeMap::from([(1, 2), (2, 1), (3, 1), (4, 1)]);
let nodes = BTreeSet::from([1, 2, 3, 4]);
let membership = Membership::new(weights, 3, nodes).unwrap();

// 1. Voter 4 leaves the weights. Retaining it keeps it as a learner. The total
// weight becomes 4, so the read weight becomes 2.
let change = Change::weights(BTreeMap::from([(1, 2), (2, 1), (3, 1)]));
let retained_change = change.clone();
let target = retained_change.apply(&membership, true).unwrap();
let dropped = change.apply(&membership, false).unwrap();
let ids: Vec<u64> = dropped.node_ids().collect();
assert_eq!(vec![1, 2, 3], ids);

// 2. Read weight 2 and write weight 3 sum to more than the total weight 4.
let intersect = target.read_write_quorums_intersect();
assert_eq!(Some(true), intersect);

// 3. The old write quorum {2,3,4} misses the new read quorum {1}, so OpenRaft
// first appends the joint. A write quorum of the joint is a write quorum of
// both configs, so {2,3,4} commits nothing in the joint.
let direct = membership.supports_direct_transition_to(&target);
assert!(!direct);
let joint = membership.joint_with(&target);
let commits = joint.write_quorum_set().is_quorum([2, 3, 4].iter());
assert!(!commits);

// 4. `make_uniform()` reduces the joint to the target, which OpenRaft appends
// directly.
let change = <Change as RaftMembershipEdit<Membership>>::make_uniform();
let uniform = change.apply(&joint, true).unwrap();
assert_eq!(target, uniform);
let direct = joint.supports_direct_transition_to(&uniform);
assert!(direct);
```

To write a membership of your own, copy the source of four parts of this
crate:

- `src/weighted/`, which holds these three types and their error type
  [`InvalidWeights`];
- `src/quorum/joint.rs`, which holds `Joint`;
- `src/config.rs`, which holds the crate-private `Config`;
- `src/errors/configs_error.rs`, which holds the crate-private `ConfigsError`
  that `Config` returns.

`Config` stores one config: its two rules, and its nodes with their metadata
and roles. `WeightedMembership` implements each required method of
`RaftMembership` with one call to a function of `Config`, such as
[`get_node()`], [`node_ids()`] and [`joint_with()`]. The grid, hierarchical
and explicit memberships use the same functions. Then replace the weight
arithmetic with the rule of your policy.

[`WeightedMembership`]: crate::WeightedMembership
[`WeightedMembership::new()`]: crate::WeightedMembership::new
[`WeightedChange`]: crate::WeightedChange
[`WeightedChange::weights()`]: crate::WeightedChange::weights
[`WeightedChange::write_weight()`]: crate::WeightedChange::write_weight
[`ThresholdMembership`]: crate::ThresholdMembership
[`threshold`]: crate::docs::threshold
[`InvalidWeights`]: crate::errors::InvalidWeights
[`MembershipError::UserDefined`]: openraft::errors::MembershipError::UserDefined
[`Raft::initialize()`]: openraft::Raft::initialize
[`Raft::change_membership()`]: openraft::Raft::change_membership
[`leader_id_adv::LeaderId`]: openraft::impls::leader_id_adv::LeaderId
[`leader_id_std::LeaderId`]: openraft::impls::leader_id_std::LeaderId
[`election_quorum`]: openraft::docs::protocol::election_quorum
[`WeightedQuorumSet`]: crate::quorum::WeightedQuorumSet
[`Joint`]: crate::quorum::Joint
[`QuorumSet`]: openraft::membership::QuorumSet
[`is_quorum()`]: openraft::membership::QuorumSet::is_quorum
[`QuorumIntersection`]: openraft::membership::QuorumIntersection
[`intersects_with()`]: openraft::membership::QuorumIntersection::intersects_with
[`RaftMembership`]: openraft::membership::RaftMembership
[`read_quorum_set()`]: openraft::membership::RaftMembership::read_quorum_set
[`write_quorum_set()`]: openraft::membership::RaftMembership::write_quorum_set
[`read_write_quorums_intersect()`]: openraft::membership::RaftMembership::read_write_quorums_intersect
[`supports_direct_transition_to()`]: openraft::membership::RaftMembership::supports_direct_transition_to
[`joint_with()`]: openraft::membership::RaftMembership::joint_with
[`get_node()`]: openraft::membership::RaftMembership::get_node
[`node_ids()`]: openraft::membership::RaftMembership::node_ids
[`RaftMembershipEdit`]: openraft::membership::RaftMembershipEdit
[`apply()`]: openraft::membership::RaftMembershipEdit::apply
[`make_uniform()`]: openraft::membership::RaftMembershipEdit::make_uniform
[`add_learner()`]: openraft::membership::RaftMembershipEdit::add_learner
[`Raft::add_learner()`]: openraft::Raft::add_learner
