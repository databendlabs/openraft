# Grid membership: a complete row elects a leader, and a voter of each row commits an entry

[`GridMembership`] places the voters of each config in a rectangular grid, one
voter per cell:

- A write quorum holds at least one voter of every row. Each row can
  contribute a voter from a different column. An entry commits once a write
  quorum stores it.
- A read quorum holds every voter of some row. A candidate becomes leader only
  with votes from a read quorum.

A grid of `r` rows and `c` columns holds `r * c` voters, but a commit needs
only `r` copies, and an election with the default leader IDs only `c` votes.
For example, in a grid of 3 rows of 3 voters, a commit needs 3 copies and an
election 3 votes, where a majority of the 9 voters is 5:

```rust
use std::collections::BTreeSet;

use openraft::membership::{QuorumSet, RaftMembership};
use openraft_membership::GridMembership;

let grid = vec![vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
let nodes = BTreeSet::from([1, 2, 3, 4, 5, 6, 7, 8, 9]);
let membership = GridMembership::<u64, ()>::new(grid, nodes).unwrap();

// {1,5,9} holds a voter of each row, from three columns, so it commits an
// entry. Row {4,5,6} elects a leader.
let commits = membership.write_quorum_set().is_quorum([1, 5, 9].iter());
assert!(commits);
let elects = membership.read_quorum_set().is_quorum([4, 5, 6].iter());
assert!(elects);

// Row {1,2,3} also holds 3 voters, but none of row {4,5,6}, so it commits
// nothing.
let commits = membership.write_quorum_set().is_quorum([1, 2, 3].iter());
assert!(!commits);
```

An application selects the type in its type config, and passes the membership
to [`Raft::initialize()`], as [`weighted`] shows. The type config sets
`Membership = GridMembership<..>` and `MembershipEdit = GridChange<..>`.

## A write quorum holds a voter of every row, so it meets every read quorum

A read quorum holds a complete row, and every write quorum holds a voter of
that row. So every read quorum shares a voter with every write quorum. Two rows
share no voter, and two write quorums, such as `{1,4,7}` and `{2,5,8}`, can
share none either. Safety needs only that reads meet writes, as
[`election_quorum`] explains.

Because every read quorum meets every write quorum, the new leader holds every
committed entry, as [`weighted`][weighted-intersection] explains.

[`GridMembership::new()`] and edits return [`InvalidGrid`] inside
[`MembershipError::UserDefined`] unless the grid has a row and a column, every
row is as long as row 0, and no voter is in two cells.

```rust
use std::collections::BTreeSet;

use openraft::errors::MembershipError;
use openraft_membership::GridMembership;
use openraft_membership::errors::InvalidGrid;

// Row 1 holds 3 voters, but row 0 holds 2, so the grid is no rectangle.
let grid = vec![vec![1, 2], vec![3, 4, 5]];
let nodes = BTreeSet::from([1, 2, 3, 4, 5]);
let res = GridMembership::<u64, ()>::new(grid, nodes);

let invalid = InvalidGrid::RaggedRow { row: 1, len: 3, column_count: 2 };
assert_eq!(Err(MembershipError::UserDefined(invalid)), res);
```

## A commit waits for a voter of each row, and an election for one row or more

A commit waits until some voter of each row stores the entry. These voters can
sit in different columns, so the leader waits only for the first voter of each
row that stores the entry.

How many votes an election needs depends on the leader ID type:

- With advanced leader IDs, [`leader_id_adv::LeaderId`], the default, an
  election needs the votes of one complete row: `c` votes.
- With standard leader IDs, [`leader_id_std::LeaderId`], the votes must form
  both a read quorum and a write quorum, because standard leader IDs allow
  only one leader per term. The votes then hold a complete row and a voter of
  each other row: `r + c - 1` votes. [`election_quorum`] explains why.

The new leader also needs a live write quorum, as
[`weighted`][weighted-election] explains.

For 12 voters:

| Grid, rows x columns | Commit copies | Election votes, advanced IDs | Election votes, standard IDs |
|----------------------|---------------|------------------------------|------------------------------|
| 3 x 4                | 3             | 4                            | 6                            |
| 4 x 3                | 4             | 3                            | 6                            |
| 2 x 6                | 2             | 6                            | 7                            |
| 1 x 12               | 1             | 12                           | 12                           |

## Commits need a live voter in every row, and elections a complete live row

A grid needs few voters for each quorum, and its rows decide which failures
stop it:

- A commit needs a live voter in every row. Failures spread over the rows do
  not stop commits. In the 3 x 3 grid above, leader 5 keeps committing with
  voters 1 and 9 after the other 6 voters fail. Only the loss of a whole row,
  all `c` of its voters, stops every commit.
- An election needs a complete live row. `r` failed voters, one in each row,
  leave no complete row, so they stop every election. The loss of a whole
  column does this.

Any `min(r, c) - 1` failed voters still leave a complete row and a live voter
in every row. So the 3 x 3 grid above survives any 2 failed voters, while a
majority of 9 voters survives any 4. In return, each commit of the grid waits
for 3 voters instead of 5, and so does each election with advanced leader IDs.

A grid of one row needs every voter to elect a leader, and lets any one voter
commit an entry alone, with one copy. Converting a node map into a
`GridMembership` fills the most square grid with `r <= c`, row by row in
ascending ID order. So a prime number of nodes gives one row: a cluster of 5
nodes initialized from a node map needs all 5 voters for every election.

## Change the grid with `GridChange`

With `MembershipEdit = GridChange<..>`, [`Raft::change_membership()`] takes a
[`GridChange`]. [`GridChange::grid()`] sets the new grid, given as its rows. A
new grid can move voters to other cells, change the dimensions, or both. A new
voter must already be a node: add it as a learner first, or with the
`add_nodes` field of the change.

```rust
use std::collections::BTreeSet;

use openraft::membership::{RaftMembership, RaftMembershipEdit};
use openraft_membership::{GridChange, GridMembership};

// Node 10 is a learner.
let grid = vec![vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
let nodes = BTreeSet::from([1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
let membership = GridMembership::<u64, ()>::new(grid, nodes).unwrap();

// Voter 10 takes the cell of voter 5, which `apply()` keeps as a learner.
let change = GridChange::grid(vec![vec![1, 2, 3], vec![4, 10, 6], vec![7, 8, 9]]);
let replaced = change.apply(&membership, true).unwrap();
let want = "{configs: [Config { read: GridQuorumSet { grid: [[1, 2, 3], [4, 10, 6], [7, 8, 9]], rule: FullRow }, \
            write: GridQuorumSet { grid: [[1, 2, 3], [4, 10, 6], [7, 8, 9]], rule: OnePerRow }, \
            nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), 2: ((), NodeRole(Campaigner | ReceiveLogs)), \
            3: ((), NodeRole(Campaigner | ReceiveLogs)), 4: ((), NodeRole(Campaigner | ReceiveLogs)), \
            5: ((), NodeRole(ReceiveLogs)), 6: ((), NodeRole(Campaigner | ReceiveLogs)), \
            7: ((), NodeRole(Campaigner | ReceiveLogs)), 8: ((), NodeRole(Campaigner | ReceiveLogs)), \
            9: ((), NodeRole(Campaigner | ReceiveLogs)), 10: ((), NodeRole(Campaigner | ReceiveLogs))} }]}";
let got = replaced.to_string();
assert_eq!(want, got);

// The old row {4,5,6} misses the new write quorum {1,7,10}, so OpenRaft first
// appends a joint config.
let direct = membership.supports_direct_transition_to(&replaced);
assert!(!direct);

// Swapping voters 4 and 6 keeps every row, so both rules accept the same sets
// as before. The grid changes, so OpenRaft still first appends a joint config.
let change = GridChange::grid(vec![vec![1, 2, 3], vec![6, 5, 4], vec![7, 8, 9]]);
let swapped = change.apply(&membership, true).unwrap();
let direct = membership.supports_direct_transition_to(&swapped);
assert!(!direct);
```

## Every new grid requires a joint configuration

OpenRaft appends a changed config directly only when every read quorum of each
config intersects every write quorum of the other. Otherwise it first appends
a joint config, as [`weighted`][weighted-joint] explains. A read quorum of the
joint config holds a complete row of each grid, and a write quorum holds a
voter of every row of both grids.

The grid membership proves this only when both memberships hold a config with
the same grid. A read quorum of one membership then holds a row of that grid,
and a write quorum of the other holds a voter of each of its rows, so they
share a voter. Every new grid therefore goes through a joint config, even one
that only moves voters within their rows and would be safe directly. Changing
only roles, adding a learner, and leaving a joint config go directly.

## The grid types connect like the weighted types, with a `FullRow` rule and a `OnePerRow` rule

[`GridMembership`] implements [`RaftMembership`], and [`GridChange`]
implements [`RaftMembershipEdit`], in the same way as the weighted types and
with the same crate-private `Config`. [`weighted`] walks through how OpenRaft
calls them during a change. It also lists what to copy to write a membership
of your own, where `src/grid/` takes the place of `src/weighted/`. The grid
differs in its rule and in how a change builds it:

- [`GridQuorumSet`] holds the grid and a [`GridRule`]. Its [`QuorumSet`] impl
  accepts a set that holds every voter of some row, under `GridRule::FullRow`,
  or at least one voter of every row, under `GridRule::OnePerRow`. Each config
  holds the `FullRow` rule as its read rule and the `OnePerRow` rule as its
  write rule.
- The [`QuorumIntersection`] impl of `GridQuorumSet` is exact when either rule
  is `FullRow`. A row meets every `OnePerRow` quorum of a grid exactly when it
  holds a complete row of that grid. Otherwise each row of that grid has a
  voter outside it, and these voters form a `OnePerRow` quorum that it misses.
  Every row of a grid holds itself, so [`read_write_quorums_intersect()`]
  returns `Some(true)` for the target of every change. The impl returns `None`
  for two `OnePerRow` rules, which OpenRaft never compares.
- [`GridChange`] builds both rules from the grid that it sets. A change that
  sets no grid keeps both rules of the last config.

[`GridMembership`]: crate::GridMembership
[`GridMembership::new()`]: crate::GridMembership::new
[`GridChange`]: crate::GridChange
[`GridChange::grid()`]: crate::GridChange::grid
[`InvalidGrid`]: crate::errors::InvalidGrid
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
[`GridQuorumSet`]: crate::quorum::GridQuorumSet
[`GridRule`]: crate::quorum::GridRule
[`QuorumSet`]: openraft::membership::QuorumSet
[`QuorumIntersection`]: openraft::membership::QuorumIntersection
[`RaftMembership`]: openraft::membership::RaftMembership
[`RaftMembershipEdit`]: openraft::membership::RaftMembershipEdit
[`read_write_quorums_intersect()`]: openraft::membership::RaftMembership::read_write_quorums_intersect
