# Extended membership change algorithm

Openraft tries to commit one or more membership logs in order to change the
membership to a specified `voter_list`. At each step, the log that it tries to
commit is:

-   The `voter_list` itself, if it is **safe** to change from the previous
    membership to `voter_list` directly.

-   Otherwise, a **joint** config of the specified `voter_list` and the
    config in the previous membership.

The algorithm used by Openraft is known as the **Extended membership change**.

> The Extended membership change algorithm used by Openraft is a more
> generalized form of the standard Raft membership change algorithm. The 2-step
> joint algorithm and the 1-step algorithm described in the Raft paper are
> specialized versions of this algorithm.

The Extended membership change algorithm provides more flexible than the original joint algorithm:

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

Here's the example, which demonstrates that it is always safe to change
membership from one to another along the edges in the following diagram:

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


## Disjoint memberships

A counter-intuitive conclusion is that:

**Even when two leaders propose two memberships without intersection, consensus
will still, be achieved**.

E.g., given the current membership to be `c1c2`,
- if `L1` proposed `c1c3`,
- and `L2` proposed `c2c4`.

There won't be a brain split problem.


## Spec of extended membership change algorithm

The Extended membership change algorithm used by Openraft requires four
constraints to work correctly:

This algorithm relies on four constraints for proper functioning:

-   (0) **use-at-once**:
    The new membership appended to the log becomes effective immediately, meaning that openraft
    uses the last seen membership config in the log, regardless of whether it is committed or not.

-   (1) **propose-after-commit**:
    A leader can propose new membership only after the previous one has been
    committed.

-   (2) **old-new-intersect** (safe transition):
    (This constraint is the only one loosened from the original Raft) Any
    quorum in the new membership (`m'`) intersects with any quorum in the old
    committed membership (`m`):

    `∀qᵢ ∈ m, ∀qⱼ ∈ m'`: `qᵢ ∩ qⱼ ≠ ø`.

-   (3) **initial-log**:
    A leader must replicate an initial empty log to a quorum in the last seen
    membership to commit all previous logs.

In our implementation, (2) **old-new-intersect** is simplified as follows:
The new membership must include a config entry identical to one in the last
committed membership.

For example, if the last committed membership is `[{a, b, c}]`, then a valid new membership could be:
a joint membership: `[{a, b, c}, {x, y, z}]`.

If the last committed membership is `[{a, b, c}, {x, y, z}]`, a valid new membership
might be: `[{a, b, c}]`, or `[{x, y, z}]`.



## Proof of correctness

Assuming a brain split issue has occurred,
there are two leaders (`L1` and `L2`) proposing different memberships (`m1` and `m2`(`mᵢ = cᵢcⱼ...`)):

`L1`: `m1`,
`L2`: `m2`

As a result, the log history of `L1` and the log history of `L2` have diverged.
Let `m0` represent the last common membership in the log histories:

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

From (1) **propose-after-commit**,
- `L1` must have committed log entry `m0` to a quorum in `m0` in `term_1`.
- `L2` must have committed log entry `m0` to a quorum in `m0` in `term_2`.

Assume `term_1 < term_2`.

From (3) **initial-log**, `L2` has at least one log with `term_2` committed in a
quorum in `m0`.

∵ (2) **old-new-intersect** and the fact that `term_1 < term_2`,

∴ log entry `m1` can never be committed by `L1`,
because log replication or voting will always see a higher `term_2` on a node in a quorum in `m0`.

For the same reason, a candidate with log entry `m1` can never become a leader.

∴ It's impossible for two leaders to both commit a log entry.

QED.
