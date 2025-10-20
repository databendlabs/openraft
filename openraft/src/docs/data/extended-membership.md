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

Although `c1c3` and `c2c4` have no common nodes, neither leader can commit their proposal:

1. Both `c1c3` and `c2c4` must satisfy **old-new-intersect** with the committed membership `c1c2`
2. To commit, each leader must obtain a quorum in both their proposed membership and the old membership `c1c2`
3. Any quorum in `c1c2` requires nodes from both `c1` and `c2`
4. This shared quorum requirement prevents both leaders from simultaneously committing conflicting memberships
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
    membership to commit all previous logs.

OpenRaft implements constraint (2) **old-new-intersect** by ensuring that
the new membership includes a config entry identical to one in the last
committed membership.

For example, if the last committed membership is `[{a, b, c}]`, then a valid new membership could be:
a joint membership: `[{a, b, c}, {x, y, z}]`.

If the last committed membership is `[{a, b, c}, {x, y, z}]`, a valid new membership
might be: `[{a, b, c}]`, or `[{x, y, z}]`.


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
