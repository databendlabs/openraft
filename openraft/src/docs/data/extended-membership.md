# Extended membership change algorithm

OpenRaft tries to commit one or more membership logs in order to change the
membership to a specified `voter_list`. At each step, the log that it tries to
commit is:

-   The `voter_list` itself, if it is **safe** to change from the previous
    membership to `voter_list` directly.

-   Otherwise, a **joint** config of the specified `voter_list` and the
    config in the previous membership.

The algorithm used by OpenRaft is known as the **Extended membership change**.

> The Extended membership change algorithm used by OpenRaft is a more
> generalized form of the standard Raft membership change algorithm. The 2-step
> joint algorithm and the 1-step algorithm described in the Raft paper are
> specialized versions of this algorithm.


## Notation

Throughout this document, the following notation is used:

- `cᵢ`: A uniform membership config, representing a single set of nodes (e.g., `{a, b, c}`)
- `cᵢcⱼ`: A joint membership, combining multiple configs (e.g., `[{a, b, c}, {x, y, z}]`)
- `mᵢ`: A membership log entry at position `i` in the log
- `qᵢ`: A quorum in membership `i`
- `Lᵢ`: Leader `i`
- `term_i`: The Raft term number `i`


## Flexibility comparison

The Extended membership change algorithm provides more flexibility than the original joint algorithm:

-   The original **Joint** algorithm in the Raft paper allows changing
    membership in an alternate pattern of joint membership and uniform
    membership. For example, the membership entry in a log history could be:

  `c1` → `c1c2` → `c2` → `c2c3` → `c3` ...

  where:
  - `cᵢ` is a uniform membership, such as `{a, b, c}`;
  - `cᵢcⱼ` is a joint of two node lists, such as `[{a, b, c}, {x, y, z}]`.


-   **Extended** algorithm:

    Extended membership change algorithm allows changing membership in the
    following way:

    `c1`  →  `c1c2c3`  →  `c3c4`  →  `c4`.

    Or revert to a previous membership:

    `c1c2c3`  →  `c1`.


## Flexibility

The following diagram demonstrates safe membership transitions. Each node represents a membership configuration, and each edge represents a valid transition satisfying **old-new-intersect**:

```text
          c3
         /  \
        /    \
       /      \
   c1c3 ------ c2c3
    / \        / \
   /   \      /   \
  /     \    /     \
c1 ----- c1c2 ----- c2
```

Every edge is bidirectional and satisfies **old-new-intersect** because the new membership includes at least one config from the old membership:
- `c1` ↔ `c1c2`: share `c1`
- `c1` ↔ `c1c3`: share `c1`
- `c2` ↔ `c1c2`: share `c2`
- `c2` ↔ `c2c3`: share `c2`
- `c3` ↔ `c1c3`: share `c3`
- `c3` ↔ `c2c3`: share `c3`
- `c1c2` ↔ `c1c3`: share `c1`
- `c1c2` ↔ `c2c3`: share `c2`
- `c1c3` ↔ `c2c3`: share `c3`


## Disjoint memberships

**Even when two leaders propose two memberships without intersection, brain split cannot occur**.

For example, given the current committed membership `c1c2`:
- `L1` proposes `c1c3`
- `L2` proposes `c2c4`

Although `c1c3` and `c2c4` have no common nodes, only one of them can be committed:

1. Both `c1c3` and `c2c4` must satisfy **old-new-intersect** with the committed membership `c1c2`
2. Say `L2` has the greater term. By (3) **initial-log**, `L2` replicates a log of its term to a quorum in `c1c2` before it proposes `c2c4`
3. Every quorum in `c1c3` holds a node of that quorum, which rejects replication from `L1` and refuses to vote for a candidate whose last log is `c1c3`
4. So `c1c3` can never be committed, and no node holding it can become a leader
5. Only one leader can successfully commit, preventing brain split


## Spec of extended membership change algorithm

This algorithm relies on four constraints for proper functioning:

-   (0) **use-at-once**:
    The new membership appended to the log becomes effective immediately, meaning that OpenRaft
    uses the last seen membership config in the log, regardless of whether it is committed or not.

-   (1) **propose-after-commit**:
    A leader can propose new membership only after the previous one has been
    committed.

-   (2) **old-new-intersect** (safe transition):
    (This constraint is the only one loosened from the original Raft) Any
    quorum in the new membership (`m'`) intersects with any quorum in the old
    committed membership (`m`):

    `∀qᵢ ∈ m, ∀qⱼ ∈ m'`: `qᵢ ∩ qⱼ ≠ ø`.

    In plain terms: for every possible quorum in the old membership and every
    possible quorum in the new membership, there must be at least one node
    that appears in both quorums. This overlap ensures that information from
    the old membership is preserved during the transition.

-   (3) **initial-log**:
    A leader must replicate an initial empty log to a quorum in the last seen
    membership to commit all previous logs, before it proposes a new
    membership.

OpenRaft implements constraint (2) **old-new-intersect** by ensuring that
a config entry of the new membership and a config entry of the last committed
membership have intersecting majorities: every majority of one intersects
every majority of the other. A config entry identical to one in the last
committed membership is the simplest such pair.

For example, if the last committed membership is `[{a, b, c}]`, then a valid new membership could be:
a joint membership: `[{a, b, c}, {x, y, z}]`, or the uniform `[{a, b, c, d}]`,
because every majority of `{a, b, c}` intersects every majority of `{a, b, c, d}`.

If the last committed membership is `[{a, b, c}, {x, y, z}]`, a valid new membership
might be: `[{a, b, c}]`, or `[{x, y, z}]`.


## Direct membership append

[`Raft::change_membership()`][] writes its target in one log entry when it can
prove (2) **old-new-intersect** for it, and reaches the target through a joint
config otherwise. [`Raft::append_membership()`][] writes the caller's exact
membership as one log entry. It must still satisfy (2) **old-new-intersect**,
and it accepts a transition only when the rule below proves that constraint.

### Two voter sets with intersecting majorities

To find two disjoint majorities of voter sets `V` and `V'`, build each one to
use as few shared node ids as possible: a majority of `V` first takes the node
ids that `V'` lacks, and a majority of `V'` first takes the node ids that `V`
lacks. Each then takes shared node ids only to reach its majority size. So two
disjoint majorities exist exactly when the shared node ids are enough for both.

Adding or removing one voter always keeps the majorities intersecting. Take
`V' = V ∪ {x}` with `|V| = n`. Any majority of `V` holds at least `⌊n/2⌋ + 1`
nodes of `V`. Any majority of `V'` holds at least `⌊(n+1)/2⌋ + 1` nodes of
`V'`, so setting `x` aside it holds at least `⌊(n+1)/2⌋` nodes of `V`. The two
counts add up to at least `n + 1`, which is more than `|V| = n`, so the two
majorities intersect. Removing one voter is the same statement with `V` and
`V'` swapped.

Replacing one voter keeps the majorities intersecting exactly when the voter set
has an even number `n` of nodes. A smallest majority of either set holds its one
node id that the other set lacks, plus `⌊n/2⌋` of the `n - 1` shared node ids.
Two such majorities fit in the shared node ids without overlap exactly when
`2⌊n/2⌋ ≤ n - 1`, that is, when `n` is odd. From `{a,b,c}` to `{b,c,d}`, the
majority `{a,b}` and the majority `{c,d}` share no node. From `{a,b,c,d}` to
`{a,b,c,x}`, every majority of either set holds at least two of `{a,b,c}`, so a
majority of one and a majority of the other always share a node.

A uniform membership has one voter set, so between two uniform memberships this
check decides (2) **old-new-intersect** exactly:

| Transition | Disjoint majorities | Result |
| --- | --- | --- |
| `[{a,b,c}]` → `[{a,b,c}]` | none | accept |
| `[{a,b,c}]` → `[{a,b,c,x}]` | none | accept |
| `[{a,b,c,x}]` → `[{a,b,c}]` | none | accept |
| `[{a,b,c,d}]` → `[{a,b,c,x}]` | none | accept |
| `[{a,b,c}]` → `[{b,c,d}]` | `{a,b}` and `{c,d}` | reject |
| `[{a,b,c}]` → `[{a,b,c,x,y}]` | `{a,b}` and `{c,x,y}` | reject |

### The rule: a pair of voter sets with intersecting majorities

The old membership `m` must hold a voter set `V` and the new membership `m'` a
voter set `V'`, such that every majority of `V` intersects every majority of
`V'`. Every quorum of a membership holds a majority of each of its voter sets,
so every `qᵢ ∈ m` and `qⱼ ∈ m'` then intersect. This is the condition the
extended algorithm already uses.

Two equal voter sets form such a pair, so the rule puts no maximum on the
number of voter sets: `c1` → `c1c2c3c4` is accepted because `c1` appears in
both. Two voter sets one node id apart form such a pair as well, by the proof
above: `[{a,b,c}, {d,e,f}]` → `[{a,b,c,x}, {d,e,f,y}]` is accepted through
`{a,b,c}` and `{a,b,c,x}`.

Shared members alone prove nothing. From `[{a,b,c}, {a,b,d}]` to `[{a,b,e}]`,
every voter set holds `a` and `b`, yet the majorities `{a,c}` of `{a,b,c}` and
`{a,d}` of `{a,b,d}` each miss the majority `{b,e}` of `{a,b,e}`. The transition
is rejected, and rightly so: the quorum `{a,c,d}` of the old membership and the
quorum `{b,e}` of the new one share no node.

When either membership has more than one voter set, the rule is conservative:
it also rejects transitions that are safe but that it cannot prove.
[`UnsupportedMembershipTransition`][] carries such an example.

### The current-term guard

The rule proves that a new membership intersects its parent. It says nothing
about two children of the same parent. Every membership change, a
direct append included, therefore enforces (3) **initial-log**, the corrected
Raft single-server rule: the leader must commit a log entry of its own term
before it appends a configuration entry. Openraft records that entry as
`Leader::noop_log_id` and the quorum-granted commit as
[`RaftState::cluster_committed()`][], and rejects the change until:

```text
cluster_committed >= leader.noop_log_id
```

Without the guard, two children proposed in different terms can both commit
while not intersecting each other. From the committed `[{a,b,c,d}]`, one leader
proposes `[{a,b,c,d,u}]` and a later leader proposes `[{a,b,c,d,v}]`. Both pass
the rule, yet the quorum `{a,b,u}` and the quorum `{c,d,v}` share no node.
[`UncommittedLeaderLog`][] walks through the full run.

[`Raft::change_membership()`][] needs the guard too. Each of its proposals
passes the rule against the parent, but two of its proposals from one parent
need not pass it against each other. From the committed `[{a,b,c}, {x,y,z}]`,
one leader proposes `[{a,b,c}]` and a later leader proposes `[{x,y,z}]`, which
share no node.


## Proof of correctness

(In this proof, ∵ means "because" and ∴ means "therefore")

**Proof by contradiction**: Assume a brain split has occurred with two leaders (`L1` and `L2`) proposing different memberships (`m1` and `m2`).

### Setup

The log histories have diverged. Let `m0` be the last common membership:

```text
L1       L2

m1       m2
 \      /
  \    o   term-2
   \   |
    `--o   term-1
       |
       m0
```

### Preconditions

From (1) **propose-after-commit**:
- `L1` committed `m0` to a quorum in `m0` in `term_1`
- `L2` committed `m0` to a quorum in `m0` in `term_2`

Assume `term_1 < term_2`.

From (3) **initial-log**: `L2` has committed at least one log with `term_2` to a quorum in `m0`.

∵ (2) **old-new-intersect** and the fact that `term_1 < term_2`,

∴ log entry `m1` can never be committed by `L1`. This is because any
attempt by `L1` to replicate or gather votes for `m1` must contact a
quorum in `m0` (due to the **old-new-intersect** constraint). At least
one node in that quorum will have seen `term_2`, which is higher than
`term_1`. This node will reject `L1`'s replication or vote request,
preventing `m1` from being committed.

For the same reason, a candidate with log entry `m1` can never become a
leader, as it cannot gather votes from a quorum in `m0` without
encountering a node with the higher `term_2`.

∴ It's impossible for two leaders to both commit a log entry.

QED.


[`Raft::append_membership()`]: `crate::Raft::append_membership`
[`Raft::change_membership()`]: `crate::Raft::change_membership`
[`RaftState::cluster_committed()`]: `crate::RaftState::cluster_committed`
[`UncommittedLeaderLog`]: `crate::errors::UncommittedLeaderLog`
[`UnsupportedMembershipTransition`]: `crate::errors::UnsupportedMembershipTransition`
