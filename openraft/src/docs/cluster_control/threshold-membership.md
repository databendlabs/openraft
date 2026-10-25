# Threshold membership: configure the copies required to commit

[`ThresholdMembership`] is a built-in membership type with a configurable
`write_size`. Each config holds a voter set and one write quorum size:

- Any `write_size` voters form a write quorum. An entry commits once a write
  quorum stores it.
- The read quorum size is `voter_count + 1 - write_size`, the minimum that
  intersects every write quorum. A candidate becomes leader only with votes
  from a read quorum.

A larger `write_size` keeps more copies of each committed entry and waits for
more voters. A smaller `write_size` allows faster commits but needs more
votes to elect a replacement leader under the default leader IDs.

An application selects the type in its type config, and passes a
`ThresholdMembership` to [`Raft::initialize()`]:

```rust,ignore
openraft::declare_raft_types!(
    pub TypeConfig:
        D = Request,
        R = Response,
        Membership = openraft::membership::ThresholdMembership<u64, BasicNode, Self::ErrorSource>,
);

// 5 voters: a commit needs 2 copies, so an election needs 4 votes.
let membership = ThresholdMembership::new(btreeset! {1, 2, 3, 4, 5}, 2, nodes)?;
raft.initialize(membership).await?;
```

## The derived read quorum intersects every write quorum

With `n` voters and write size `w`, a write quorum leaves at most `n - w`
voters outside it. Any set of `n - w + 1` voters therefore contains a voter
from every write quorum. A smaller read quorum could fit entirely outside a
write quorum. For example, with 5 voters and write size 2, the write quorum
`{4,5}` misses the set `{1,2,3}`, so an election needs at least 4 votes.

Every election thus reaches a voter that stores each committed entry. That
voter votes only for a candidate whose log is at least as up to date as its
own, so the new leader holds every committed entry.

[`ThresholdMembership::new()`] requires `1 <= write_size <= voter_count`.
It rejects an invalid write size with [`InvalidThresholds`] inside
[`MembershipError::UserDefined`]. A membership change validates the write
size in the same way.

```rust
use std::collections::BTreeSet;

use openraft::membership::ThresholdMembership;

let voters = BTreeSet::from([1, 2, 3, 4, 5]);

// Write size 2 derives read size 4.
let valid = ThresholdMembership::<u64, ()>::new(voters.clone(), 2, voters.clone());
assert!(valid.is_ok());

// Six copies cannot be stored on five voters.
let invalid = ThresholdMembership::<u64, ()>::new(voters.clone(), 6, voters);
assert!(invalid.is_err());
```

## A commit waits for `write_size` voters, and an election for the derived read size or more votes

A commit waits until `write_size` voters store the entry, the leader
included. So the leader waits only for its fastest `write_size - 1`
followers, and the cluster keeps committing while `write_size` voters are up.

How many votes an election needs depends on the leader ID type:

- With advanced leader IDs, [`leader_id_adv::LeaderId`], the default, an
  election needs `read_size = voter_count + 1 - write_size` votes.
- With standard leader IDs, [`leader_id_std::LeaderId`], an election needs
  `max(read_size, write_size)` votes. Standard leader IDs allow only one
  leader per term, so the votes must form both a read quorum and a write
  quorum. [`election_quorum`] explains why.

The new leader also needs a live write quorum. It completes a write or a
linearizable read only after it commits the blank entry of its term.

For 5 voters:

| `write_size` | Derived read size / election votes, advanced IDs | Election votes, standard IDs |
|--------------|--------------------------------------------------|------------------------------|
| 3            | 3                                                | 3                            |
| 2            | 4                                                | 4                            |
| 4            | 2                                                | 4                            |

- Write size 3 behaves like the default [`Membership`] on these 5 voters:
  a commit and an election each need a majority, so the cluster survives
  2 failed voters.
- Write size 2 commits with 2 copies, so a leader keeps committing while
  only one follower is up. After the leader fails, all 4 other voters
  must be up to elect a new one.
- Write size 4 needs 4 copies to commit. With advanced leader IDs, it
  elects a leader with 2 votes, but that leader commits nothing until
  4 voters are up.

Converting a node map into `ThresholdMembership` defaults to a majority
write size. With 4 voters, that gives write size 3 and read size 2;
standard leader IDs still need 3 votes for an election.

## Change the write size with `ThresholdChange`

[`Raft::change_membership()`] takes a [`ThresholdChange`]:

```rust,ignore
// Require 4 copies and derive read size 2, keeping the 5 voters.
raft.change_membership(ThresholdChange::write_size(4), false).await?;

// Remove voter 5 and require 3 copies; the derived read size stays 2.
let remove = ChangeMembers::RemoveVoters(btreeset! {5});
raft.change_membership(ThresholdChange::members(remove).with_write_size(3), false).await?;
```

A [`ChangeMembers`] value also works. It keeps `write_size`, while the new
voter count determines the read size. Growing from 3 voters to 5 with write
size 2 raises the read size from 2 to 4. Removing voters preserves the
configured number of copies: shrinking from 5 voters to 3 with write size 4
fails unless the same change sets a smaller write size.

## Changing the write size on the same voters requires a joint configuration

During a change, some nodes still use the old config while others already
use the new one. OpenRaft appends the new config directly only when every
read quorum of each config intersects every write quorum of the other.
Otherwise it first appends a joint config, whose quorums satisfy both
configs, and then the new config.

Changing `write_size` on the same voters always needs a joint config:
raising the write size lowers the read size, so the old write quorums may
miss the new read quorums. Lowering it causes the reverse problem. Keeping
the same voters and write size needs no joint config.

For example, switching 5 voters from write size 2 to write size 4 changes
the read size from 4 to 2. Without a joint config, an old leader could
commit on `{1,2}`, while a candidate using the new config wins votes from
`{3,4}` and misses that committed entry.

[`ThresholdMembership`]: crate::membership::ThresholdMembership
[`ThresholdMembership::new()`]: crate::membership::ThresholdMembership::new
[`ThresholdChange`]: crate::membership::ThresholdChange
[`InvalidThresholds`]: crate::membership::InvalidThresholds
[`MembershipError::UserDefined`]: crate::errors::MembershipError::UserDefined
[`Membership`]: crate::Membership
[`ChangeMembers`]: crate::ChangeMembers
[`Raft::initialize()`]: crate::Raft::initialize
[`Raft::change_membership()`]: crate::Raft::change_membership
[`leader_id_adv::LeaderId`]: crate::impls::leader_id_adv::LeaderId
[`leader_id_std::LeaderId`]: crate::impls::leader_id_std::LeaderId
[`election_quorum`]: crate::docs::protocol::election_quorum
