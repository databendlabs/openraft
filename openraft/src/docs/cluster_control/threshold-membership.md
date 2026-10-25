# Threshold membership: quorums of any `read_size` or any `write_size` voters

[`ThresholdMembership`] is a built-in membership type with separate read and
write quorums. Each config holds a voter set and two sizes:

- Any `read_size` voters form a read quorum. A candidate becomes leader only
  with votes from a read quorum.
- Any `write_size` voters form a write quorum. An entry commits once a write
  quorum stores it.

A small `write_size` gives fast commits. A small `read_size` lets the
cluster elect a leader with fewer votes, under the default leader IDs. The
two sizes must add up to more than the number of voters.

An application selects it in the type config and passes a
`ThresholdMembership` to [`Raft::initialize()`]:

```rust,ignore
openraft::declare_raft_types!(
    pub TypeConfig:
        D = Request,
        R = Response,
        Membership = openraft::membership::ThresholdMembership<u64, BasicNode>,
);

// 5 voters: an election needs 4 votes, and a commit needs 2 copies.
let membership = ThresholdMembership::new(btreeset! {1, 2, 3, 4, 5}, 4, 2, nodes)?;
raft.initialize(membership).await?;
```

## `read_size + write_size > voter_count` keeps every committed entry

With `n` voters, any `read_size` voters and any `write_size` voters share at
least one voter when `read_size + write_size > n`. When the sum is `n` or
less, a read quorum and a write quorum can be disjoint: with 5 voters,
`read_size = 3` and `write_size = 2` allow the read quorum `{1,2,3}` and the
write quorum `{4,5}`.

So under the rule, every election reaches a voter that stores each committed
entry. That voter votes only for a candidate whose log is at least as up to
date as its own, so the new leader holds every committed entry.

[`ThresholdMembership::new()`] rejects sizes that break the rule, and sizes
that are 0 or greater than `n`, with [`InvalidThresholds`] inside
[`MembershipError::UserDefined`]. A membership change to such sizes fails the
same way.

```rust
use std::collections::BTreeSet;

use openraft::membership::ThresholdMembership;

let voters = BTreeSet::from([1, 2, 3, 4, 5]);

// 4 + 2 > 5: valid.
let valid = ThresholdMembership::<u64, ()>::new(voters.clone(), 4, 2, voters.clone());
assert!(valid.is_ok());

// 3 + 2 = 5: rejected.
let invalid = ThresholdMembership::<u64, ()>::new(voters.clone(), 3, 2, voters);
assert!(invalid.is_err());
```

## A commit waits for `write_size` voters, and an election for `read_size` or more votes

A commit waits until `write_size` voters store the entry, the leader
included. So the leader waits only for its fastest `write_size - 1`
followers, and the cluster keeps committing while `write_size` voters are up.

How many votes an election needs depends on the leader ID type:

- With advanced leader IDs, [`leader_id_adv::LeaderId`], the default, an
  election needs `read_size` votes.
- With standard leader IDs, [`leader_id_std::LeaderId`], an election needs
  `max(read_size, write_size)` votes. Standard leader IDs allow only one
  leader per term, so the votes must form both a read quorum and a write
  quorum. [`election_quorum`] explains why.

So the cluster replaces a failed leader only while enough voters for an
election are up. The new leader also needs a live write quorum. It completes
a write or a linearizable read only after it commits the blank entry of its
term.

For 5 voters:

| `read_size`, `write_size` | Copies to commit | Votes to elect, advanced IDs | Votes to elect, standard IDs |
|---------------------------|------------------|------------------------------|------------------------------|
| 3, 3                      | 3                | 3                            | 3                            |
| 4, 2                      | 2                | 4                            | 4                            |
| 2, 4                      | 4                | 2                            | 4                            |

- (3,3) behaves like the default [`Membership`]: a commit and an election
  each need a majority, so the cluster survives 2 failed voters.
- (4,2) commits with 2 copies, so a leader keeps committing while only one
  follower is up. But an election needs 4 votes: after the leader fails, all
  4 other voters must be up to elect a new one.
- (2,4) needs 4 copies to commit, so the cluster stops committing when 2
  voters are down. With advanced leader IDs, it elects a leader with 2
  votes, but that leader commits nothing until 4 voters are up.

## Change the sizes with `ThresholdChange`

With this membership type, [`Raft::change_membership()`] takes a
[`ThresholdChange`]:

```rust,ignore
// Change only the sizes, and keep the voters.
raft.change_membership(ThresholdChange::sizes(2, 4), false).await?;

// Remove voter 5, and set sizes that fit the 4 voters left.
let remove = ChangeMembers::RemoveVoters(btreeset! {5});
raft.change_membership(ThresholdChange::members(remove).with_sizes(2, 3), false).await?;
```

A [`ChangeMembers`] value works as well. It converts into a `ThresholdChange`
that keeps both sizes.

### OpenRaft appends a joint config first when old and new quorums can miss each other

During a change, some nodes still use the old config while others already
use the new one. OpenRaft appends the new config as one log entry only when
every read quorum of each config intersects every write quorum of the other.
With the same `n` voters, this means `old_read_size + new_write_size > n`
and `old_write_size + new_read_size > n`. Otherwise OpenRaft first appends a
joint config, in which a quorum must be a quorum of both configs, and then
the new config.

For example, switching 5 voters from (4,2) to (2,4) needs a joint config,
because `old_write_size + new_read_size = 2 + 2`, which is not more than 5.
Without it, a leader that still uses the old config could commit an entry on
2 voters, while a node that holds the new config wins an election with the
votes of 2 other voters. The new leader would then miss the committed entry.

A change that keeps the voters and lowers neither size needs no joint
config: both sums stay at least `old_read_size + old_write_size`, which is
more than `n`.

### A change of the voter count must also set the sizes

A `ThresholdChange` keeps both sizes unless it sets them, but the same sizes
mean something else for another voter count. A change from 5 to 6 voters
that keeps (3,3) fails with [`InvalidThresholds`], because `3 + 3` is not
more than 6. Sizes that still fit can change the trade-off: (2,4) on 4
voters makes every commit wait for all 4 voters. So set the sizes in the
same change, with [`ThresholdChange::with_sizes()`].

[`ThresholdMembership`]: crate::membership::ThresholdMembership
[`ThresholdMembership::new()`]: crate::membership::ThresholdMembership::new
[`ThresholdChange`]: crate::membership::ThresholdChange
[`ThresholdChange::with_sizes()`]: crate::membership::ThresholdChange::with_sizes
[`InvalidThresholds`]: crate::membership::InvalidThresholds
[`MembershipError::UserDefined`]: crate::errors::MembershipError::UserDefined
[`Membership`]: crate::Membership
[`ChangeMembers`]: crate::ChangeMembers
[`Raft::initialize()`]: crate::Raft::initialize
[`Raft::change_membership()`]: crate::Raft::change_membership
[`leader_id_adv::LeaderId`]: crate::impls::leader_id_adv::LeaderId
[`leader_id_std::LeaderId`]: crate::impls::leader_id_std::LeaderId
[`election_quorum`]: crate::docs::protocol::election_quorum
