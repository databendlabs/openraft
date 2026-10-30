# Explicit membership: list the minimal quorums of each config

[`ExplicitMembership`] lets each config list its minimal read quorums and its
minimal write quorums:

- A set of voters is a write quorum when it holds every voter of a listed write
  quorum. An entry commits once a write quorum stores it.
- A set of voters is a read quorum when it holds every voter of a listed read
  quorum. A candidate becomes leader only with votes from a read quorum.

Lists state any policy, including one that no size, weight, group or grid rule
states. The two lists need not name the same voters. For example, a backup
voter can be part of every write quorum but of no read quorum. Every commit
then waits for a copy at the backup site, while an election with the default
leader IDs needs only primary voters:

```rust
use std::collections::BTreeSet;

use openraft::membership::{QuorumSet, RaftMembership};
use openraft_membership::ExplicitMembership;

// A write needs two of the primary voters 1, 2 and 3, and the backup voter 9.
let write = [
    BTreeSet::from([1, 2, 9]),
    BTreeSet::from([1, 3, 9]),
    BTreeSet::from([2, 3, 9]),
];
// A read needs two of the primary voters.
let read = [BTreeSet::from([1, 2]), BTreeSet::from([1, 3]), BTreeSet::from([2, 3])];
let nodes = BTreeSet::from([1, 2, 3, 9]);
let membership = ExplicitMembership::<u64, ()>::new(read, write, nodes).unwrap();

let commits = membership.write_quorum_set().is_quorum([1, 3, 9].iter());
assert!(commits);
let commits = membership.write_quorum_set().is_quorum([1, 2, 3].iter());
assert!(!commits);

// Voter 9 is a write voter, but no read quorum holds it.
let backup = membership.get_node(&9).unwrap();
assert!(backup.is_write_voter);
assert!(!backup.is_read_voter);
```

An application selects the type in its type config, and passes the membership
to [`Raft::initialize()`], as [`weighted`] shows. The type config sets
`Membership = ExplicitMembership<..>` and `MembershipEdit = ExplicitChange<..>`.

## Construction checks that every listed read quorum meets every listed write quorum

Every read quorum holds a listed read quorum, and every write quorum holds a
listed write quorum. So every read quorum meets every write quorum exactly when
every listed read quorum shares a voter with every listed write quorum.
[`ExplicitMembership::new()`] and every change check each such pair.

Because every read quorum meets every write quorum, the new leader holds every
committed entry, as [`weighted`][weighted-intersection] explains.

Construction and changes sort each list, drop duplicate quorums, and drop each
quorum that holds another listed quorum. [`ExplicitMembership::new()`] and
edits return [`InvalidQuorumLists`] inside [`MembershipError::UserDefined`]
when a list holds no quorum or an empty quorum, or when a listed read quorum
and a listed write quorum share no voter.

```rust
use std::collections::BTreeSet;

use openraft::errors::MembershipError;
use openraft_membership::ExplicitMembership;
use openraft_membership::errors::InvalidQuorumLists;

// Read quorum {1} misses write quorum {2,3}, so an election could miss a commit.
let read = [BTreeSet::from([1])];
let write = [BTreeSet::from([2, 3])];
let nodes = BTreeSet::from([1, 2, 3]);
let res = ExplicitMembership::<u64, ()>::new(read, write, nodes);

let invalid = InvalidQuorumLists::DisjointQuorums {
    read: BTreeSet::from([1]),
    write: BTreeSet::from([2, 3]),
};
assert_eq!(Err(MembershipError::UserDefined(invalid)), res);
```

## A commit waits for a listed write quorum, and an election for a listed read quorum or more

A commit waits until every voter of one listed write quorum stores the entry.
In the example above, a leader in the primary site waits for one more primary
voter and for voter 9.

How many votes an election needs depends on the leader ID type:

- With advanced leader IDs, [`leader_id_adv::LeaderId`], the default, an
  election needs the votes of every voter of one listed read quorum: two
  primary voters in the example.
- With standard leader IDs, [`leader_id_std::LeaderId`], the votes must form
  both a read quorum and a write quorum, because standard leader IDs allow only
  one leader per term. In the example, every election then also needs the vote
  of voter 9. [`election_quorum`] explains why.

The new leader also needs a live write quorum, as
[`weighted`][weighted-election] explains.

Voter 9 has the default voter role, so it may start an election and lead. To
keep the leader in the primary site, give voter 9 only `ReceiveLogs`, as the
node roles section of [`docs`] describes.

## Each listed quorum needs all of its voters, and long lists cost time

A listed quorum counts only while every one of its voters is up. In the example
above, the loss of voter 9 stops every commit, and the loss of two primary
voters stops every commit and every election.

A policy with many minimal quorums makes each list long: a majority of `n`
voters has `C(n, n/2 + 1)` minimal quorums, 126 for 9 voters and 6435 for 15.
Construction and each change compare every listed read quorum with every listed
write quorum. That costs `O(r * w * n)` for `r` read quorums and `w` write
quorums of at most `n` voters. [`ThresholdMembership`] states a majority with
one number.

Converting a node map into an `ExplicitMembership` lists the set of all voters
as the only read quorum and the only write quorum. Every election and every
commit then needs every voter.

## Change the lists with `ExplicitChange`

With `MembershipEdit = ExplicitChange<..>`, [`Raft::change_membership()`] takes
an [`ExplicitChange`]. [`ExplicitChange::quorums()`] sets both lists, and
[`ExplicitChange::read_quorums()`] and [`ExplicitChange::write_quorums()`] each
set one list and keep the other. A new voter must already be a node: add it as
a learner first, or with the `add_nodes` field of the change.

```rust
use std::collections::BTreeSet;

use openraft::membership::{RaftMembership, RaftMembershipEdit};
use openraft_membership::{ExplicitChange, ExplicitMembership};

let pairs = vec![BTreeSet::from([1, 2]), BTreeSet::from([1, 3]), BTreeSet::from([2, 3])];
let with_backup = vec![
    BTreeSet::from([1, 2, 9]),
    BTreeSet::from([1, 3, 9]),
    BTreeSet::from([2, 3, 9]),
];
// Node 4 is a learner.
let nodes = BTreeSet::from([1, 2, 3, 4, 9]);
let membership = ExplicitMembership::<u64, ()>::new(pairs.clone(), with_backup, nodes).unwrap();

// Stop waiting for the backup copy. Every new write quorum is a pair of primary
// voters, which meets every read quorum, old and new. The write list changes,
// so OpenRaft still first appends a joint config.
let change = ExplicitChange::write_quorums(pairs);
let without_backup = change.apply(&membership, true).unwrap();
let direct = membership.supports_direct_transition_to(&without_backup);
assert!(!direct);

// Learner 4 replaces voter 3. The old read quorum {1,3} misses the new write
// quorum {2,4}, so OpenRaft first appends a joint config.
let pairs = vec![BTreeSet::from([1, 2]), BTreeSet::from([1, 4]), BTreeSet::from([2, 4])];
let change = ExplicitChange::quorums(pairs.clone(), pairs);
let replaced = change.apply(&without_backup, true).unwrap();
let want = "{configs: [Config { read: ExplicitQuorumSet { quorums: [{1, 2}, {1, 4}, {2, 4}] }, \
            write: ExplicitQuorumSet { quorums: [{1, 2}, {1, 4}, {2, 4}] }, \
            nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), 2: ((), NodeRole(Campaigner | ReceiveLogs)), \
            3: ((), NodeRole(ReceiveLogs)), 4: ((), NodeRole(Campaigner | ReceiveLogs)), \
            9: ((), NodeRole(ReceiveLogs))} }]}";
let got = replaced.to_string();
assert_eq!(want, got);
let direct = without_backup.supports_direct_transition_to(&replaced);
assert!(!direct);
```

## Every change of a list requires a joint configuration

OpenRaft appends a changed config directly only when every read quorum of each
config intersects every write quorum of the other. Otherwise it first appends
a joint config, as [`weighted`][weighted-joint] explains.

The explicit membership proves this only when both memberships hold a config
with the same two lists. A read quorum of one membership then holds a listed
read quorum of that config, and a write quorum of the other holds a listed
write quorum of it. Construction checked that these two share a voter. Every
change of a list therefore goes through a joint config, even one that only
adds voters to listed quorums and would be safe directly. Changing only roles,
adding a learner, and leaving a joint config go directly.

## The explicit types connect like the weighted types, with two lists checked pair by pair

[`ExplicitMembership`] implements [`RaftMembership`], and [`ExplicitChange`]
implements [`RaftMembershipEdit`], in the same way as the weighted types and
with the same crate-private `Config`. [`weighted`] walks through how OpenRaft
calls them during a change. It also lists what to copy to write a membership
of your own, where `src/explicit/` takes the place of `src/weighted/`. The
explicit membership differs in its rule and in how a change builds it:

- [`ExplicitQuorumSet`] holds a list of minimal quorums. Its [`QuorumSet`]
  impl accepts a set that holds every voter of a listed quorum. Each config
  holds its read list as its read rule and its write list as its write rule.
- The [`QuorumIntersection`] impl of `ExplicitQuorumSet` is exact. It returns
  `Some(false)` when a listed quorum of one rule and a listed quorum of the
  other share no voter, and `Some(true)` otherwise.
- [`ExplicitChange`] takes each list that it sets, and keeps the other list of
  the last config. Like [`ExplicitMembership::new()`], it then compares every
  listed read quorum with every listed write quorum, and returns
  [`InvalidQuorumLists`] for a pair that shares no voter. So a change with
  disjoint lists fails in `apply()`, with an error that names the two quorums,
  before OpenRaft checks [`read_write_quorums_intersect()`].

[`ExplicitMembership`]: crate::ExplicitMembership
[`ExplicitMembership::new()`]: crate::ExplicitMembership::new
[`ExplicitChange`]: crate::ExplicitChange
[`ExplicitChange::quorums()`]: crate::ExplicitChange::quorums
[`ExplicitChange::read_quorums()`]: crate::ExplicitChange::read_quorums
[`ExplicitChange::write_quorums()`]: crate::ExplicitChange::write_quorums
[`ThresholdMembership`]: crate::ThresholdMembership
[`docs`]: crate::docs
[`InvalidQuorumLists`]: crate::errors::InvalidQuorumLists
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
[`ExplicitQuorumSet`]: crate::quorum::ExplicitQuorumSet
[`QuorumSet`]: openraft::membership::QuorumSet
[`QuorumIntersection`]: openraft::membership::QuorumIntersection
[`RaftMembership`]: openraft::membership::RaftMembership
[`RaftMembershipEdit`]: openraft::membership::RaftMembershipEdit
[`read_write_quorums_intersect()`]: openraft::membership::RaftMembership::read_write_quorums_intersect
