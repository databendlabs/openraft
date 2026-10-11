# Election quorum

A membership may define separate read and write quorums. The voters that
grant a candidate's vote form its vote set. The vote sets that win an election
depend on the leader ID mode:

- With [standard leader IDs][`leader_id_std::LeaderId`], a vote set wins only
  when it is **both a read quorum and a write quorum**.
- With [advanced leader IDs][`leader_id_adv::LeaderId`], a vote set wins when
  it is **a read quorum**.

Standard leader IDs need any two winning vote sets to share a voter, but two
read quorums may share none. Advanced leader IDs are totally ordered, so they
do not need this overlap.

## Read quorums are guaranteed to intersect only write quorums

A membership defines two families of voter sets:

- A **read quorum** is a set of voters that an election must reach. A voter
  grants its vote only to a candidate whose log is at least as up to date as
  its own, so the election reads the log state of these voters.
- A **write quorum** is a set of voters that must store a log entry before
  the leader commits it.

The membership guarantees one rule: every read quorum intersects every write
quorum. So every election reaches at least one voter that stores each
committed entry, and the new leader's log holds every committed entry.

The rule says nothing about two read quorums, or about two write quorums. For
example, place 4 voters in a 2x2 grid. Each row is a read quorum, and each
column is a write quorum:

```text
            write {1,3}   write {2,4}
read {1,2}       1             2
read {3,4}       3             4
```

Every row shares a voter with every column, so the membership is valid. The
two rows `{1,2}` and `{3,4}` share no voter.

## With standard leader IDs, a read quorum alone can elect two leaders in one term

Suppose an election needed only a read quorum. With the grid membership and
[standard leader IDs][`leader_id_std::LeaderId`], the following run commits
two different entries under one log ID:

1. Nodes 1 and 4 start elections in term `t` with equally up-to-date logs.
   Node 1 receives votes from `{1,2}`, and node 4 receives votes from
   `{3,4}`. Both vote sets are read quorums, so both nodes become leaders of
   term `t`.

2. Node 1 sends entry `x` at index `k` to node 3. Node 4 sends entry `y` at
   index `k` to node 2. Each of these AppendEntries arrives before any
   AppendEntries from the other leader. Node 3 voted for node 4, but it
   accepts `x`: the leader IDs of node 1 and node 4 are incomparable, and
   node 1 sends a committed vote. A committed vote overrides an incomparable
   uncommitted vote, as [`leader_id`] explains. Node 2 accepts `y` in the
   same way.

3. `{1,3}` and `{2,4}` are write quorums, so node 1 commits `x` and node 4
   commits `y`. A standard committed leader ID keeps only the term, so both
   entries have the log ID `(t, k)`.

Raft assumes that two entries with the same log ID are the same entry. After
step 3, node 1 and node 4 apply different commands at index `k`.

Step 1 breaks the assumption behind the vote ordering in step 2: two
incomparable leader IDs cannot both be granted. This assumption holds only
when any two winning vote sets share a voter.

## Standard leader IDs need both a read quorum and a write quorum

The election rule for standard leader IDs is:

```text
wins(vote_set) = is_read_quorum(vote_set) AND is_write_quorum(vote_set)
```

Take two winning vote sets `A` and `B`. `A` is a read quorum and `B` is a
write quorum, so they share a voter. A voter grants its vote to only one
candidate per term, so at most one candidate wins a term. This is Raft's
Election Safety.

A winning vote set is still a read quorum. So it still intersects every write
quorum, and the new leader still holds every committed entry.

The rule can need more votes than a read quorum alone. In the grid membership,
the winning vote sets are exactly the sets with at least 3 of the 4 voters:

```text
{1,2,3}, {1,2,4}, {1,3,4}, {2,3,4}, {1,2,3,4}
```

When a read quorum is any `read_size` voters and a write quorum is any
`write_size` voters, an election needs `max(read_size, write_size)` votes.

## Read and write quorums suffice for standard elections

The read-and-write rule above is one way to make any two winning vote sets
share a voter. A majority of all voters is another way, because any two
majorities intersect:

```text
wins(vote_set) = is_read_quorum(vote_set) AND is_majority(vote_set)
```

Each winning vote set of this read-and-majority rule is still a read quorum, so
the new leader still holds every committed entry. In the grid membership, both
rules accept the same vote sets, but they differ in other layouts.

OpenRaft uses the read-and-write rule so a candidate can win when both a read
quorum and a write quorum are reachable. An extra majority of all voters is
unnecessary.

An existing leader needs only a live write quorum to keep committing. For
example, with 5 voters, a read quorum of any 4 and a write quorum of any 2
satisfy the intersection rule. With only 2 voters live, including the leader,
commits can continue, but an election cannot reach a read quorum.

The read-and-majority rule requires a live majority for elections, and it
changes availability in two ways:

- The majority requirement blocks elections in grid and tree layouts, whose
  quorums are smaller than a majority. In a 4x4 grid where each row is a read
  quorum and each column is a write quorum, the read-and-write rule needs 7
  votes: one row plus one column. The read-and-majority rule needs 9 of the 16
  votes. If only the 7 voters of one row and one column are up, the cluster
  could still commit, but only the read-and-write rule elects a leader.

- The read-and-majority rule needs fewer votes only where the extra leaders
  cannot commit. Take 5 voters, where a read quorum is any 2 voters and a write
  quorum is any 4. The read-and-majority rule elects with 3 votes, and the
  read-and-write rule needs 4. A leader elected while only 3 voters are up has
  no live write quorum. It cannot commit even the blank entry of its term, so
  it serves neither writes nor linearizable reads.

## Advanced leader IDs need only a read quorum

[Advanced leader IDs][`leader_id_adv::LeaderId`] are totally ordered, for
example `(t, 1) < (t, 4)`, like Paxos ballots. A voter that granted a leader
ID rejects every smaller leader ID, including its log entries. So the run above
fails at step 2: node 3 granted `(t, 4)`, so it rejects `x` from `(t, 1)`.

Two leaders may still win one term, but every later leader keeps each
committed entry. Take an entry `e` that leader `L1` committed through a write
quorum `Q`, and a later candidate `L2` with a greater leader ID and a read
quorum `A` as its vote set:

1. `A` and `Q` share a voter `v`, because every read quorum intersects every
   write quorum.
2. `v` stored `e` before it granted `L2`. After granting `L2`, `v` rejects
   entries from the smaller leader ID of `L1`.
3. `v` grants `L2` only if the last log ID of `L2` is at least the last log
   ID of `v`. As in Raft's proof of Leader Completeness, the log of `L2` then
   holds `e`.

This proof needs only the intersection of read and write quorums. In the grid
membership, `{1,2}` or `{3,4}` alone elects a candidate.

## OpenRaft picks the rule from the leader ID type

[`RaftLeaderId::TOTALLY_ORDERED`] selects the rule.
[`leader_id_adv::LeaderId`] sets it to `true`, so a candidate counts votes with
the read quorum alone. [`leader_id_std::LeaderId`] keeps the default `false`,
so a candidate counts votes with [`BothQuorums`] from the quorum-set crate. A
custom leader ID type also gets `false`. It may set `true` only if any two of
its values are comparable.

A candidate builds the rule from the read and write quorums of the effective
membership. Elections and [Pre-Vote](`crate::docs::protocol::pre_vote`) rounds
use the same rule. After the election, the leader uses the write quorum to
commit log entries and to compute its quorum lease (see
[`check_quorum`](`crate::docs::protocol::check_quorum`)).

[`Membership`] uses one majority rule for both families. For it, both election
rules are the usual majority rule, with a majority of every configuration in a
joint membership. So elections behave as in standard Raft.

## References

- [Raft](https://raft.github.io/raft.pdf), §5.2–5.4: Election Safety, Log
  Matching, and Leader Completeness.
- [Flexible Paxos](https://arxiv.org/abs/1608.06696): phase-1 quorums need to
  intersect only phase-2 quorums, because proposals carry unique ordered
  ballots.
- [FlexiRaft](https://www.cidrdb.org/cidr2023/papers/p83-yadav.pdf), §4:
  election quorums intersect each other and every commit quorum.
- [Issue #1395](https://github.com/databendlabs/openraft/issues/1395):
  separate read and write quorums in OpenRaft.

[`leader_id`]: crate::docs::data::leader_id
[`leader_id_std::LeaderId`]: crate::impls::leader_id_std::LeaderId
[`leader_id_adv::LeaderId`]: crate::impls::leader_id_adv::LeaderId
[`RaftLeaderId::TOTALLY_ORDERED`]: crate::vote::RaftLeaderId::TOTALLY_ORDERED
[`BothQuorums`]: https://docs.rs/quorum-set/latest/quorum_set/struct.BothQuorums.html
[`Membership`]: crate::Membership
