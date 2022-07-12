# Dynamic Membership

Unlike the original raft, openraft treats all membership as a **joint** membership.
A uniform config is just a special case of joint: the joint of only one config.

Openraft offers these mechanisms for controlling member node lifecycle:

## `Raft::add_learner()`

This method will add a learner to the cluster,
and immediately begin syncing logs from the leader.

- A **Learner** won't vote for leadership.


## `Raft::change_membership(node_list, turn_to_learner)`

This method will initiate a membership change and returns when the effective
membership becomes `node_list` and committed.

If there are nodes in the given membership that is not a `Learner`, this method will fail.
Thus the application should always call `Raft::add_learner` first.

Once the new membership is committed, whether or not a `Voter` that is not in the new membership will
revert to a `Learner` is base on the `turn_to_learner`.

If `turn_to_learner` is true, then all the members which not exists in the new membership,
will be turned into learners, otherwise will be removed.

Example of `turn_to_learner` usage:
If the original membership is {"members":{1,2,3}, "learners":{}}, and call `change_membership` 
with `node_list` {3,4,5}, then:
    - If `turn_to_learner` is true, after commit the new membership is {"members":{3,4,5}, "learners":{1,2}}.
    - Otherwise if `turn_to_learner` is false, then the new membership is {"members":{3,4,5}, "learners":{}}, 
      in which the members not exists in the new membership just be removed from the cluster.

## Extended membership change algo

Openraft tries to commit one or more membership logs to finally change the
membership to `node_list`.
In every step, the log it tries to commit is:

-   the `node_list` itself, if it is safe to change from previous membership to
    `node_list` directly.

-   otherwise, a **joint** of the specified `node_list` and one config in the
    previous membership.


This algo that openraft uses is the so-called **Extended membership change**.

> It is a more generalized form of membership change.
> The original 2-step **joint** algo and 1-step algo in raft-paper are all specialized versions of this algo.


This algo provides more flexible membership change than the original joint algo:

-   The original **Joint** algo:

    The original **joint** algo in raft-paper allows changing membership in an
    alternate pattern of joint membership and uniform membership.
    E.g.,  the membership entry in a log history could be:

    `c1`  →  `c1c2`  →  `c2`  →  `c2c3`  →  `c3`  ...

    Where:
    - `cᵢ` is a uniform membership, such as `{a, b, c}`;
    - `cᵢcⱼ` is a joint of two node lists, such as `[{a, b, c}, {x, y, z}]`.

-   **Extended** algo:

    Extended membership change algo allows changing membership in the
    following way:

    `c1`  →  `c1c2c3`  →  `c3c4`  →  `c4`.

    Or revert to a previous membership:

    `c1c2c3`  →  `c1`.


#### Flexibility

Another example shows that it is always safe to change membership from one
to another along the edges in the following diagram:

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

#### Disjoint memberships

A counter-intuitive conclusion is that:

**Even when two leaders propose two memberships without intersection, consensus will
still, be achieved**.

E.g., given the current membership to be `c1c2`, if
`L1` proposed `c1c3`,
`L2` proposed `c2c4`.

There won't be a brain split problem.



#### Spec of extended membership change algo

This algo requires four constraints to work correctly:


-   (0) **use-at-once**:
    The new membership that is appended to log will take effect at once, i.e., openraft
    uses the last seen membership config in the log, no matter it is committed or not.


-   (1) **propose-after-commit**:
    A leader is allowed to propose new membership only when the previous one is
    committed.


-   (2) **old-new-intersect**(safe transition):
    (This is the only constraint that is loosened from the original raft) Any
    quorum in new membership(`m'`) intersect with any quorum in the old
    committed membership(`m`):

    `∀qᵢ ∈ m, ∀qⱼ ∈ m'`: `qᵢ ∩ qⱼ ≠ ø`.


-   (3) **initial-log**:
    A leader has to replicate an initial blank log to a quorum in last seen
    membership to commit all previous logs.



In our implementation, (2) **old-new-intersect** is simplified to:
The new membership has to contain a config entry that is the same as one in the last
committed membership.

E.g., given the last committed one is `[{a, b, c}]`, then a valid new membership may be:
a joint membership: `[{a, b, c}, {x, y, z}]`.

If the last committed one is `[{a, b, c}, {x, y, z}]`, a valid new membership
may be: `[{a, b, c}]`, or `[{x, y, z}]`.



### Proof of correctness

Assumes there was a brain split problem occurred,
then there are two leaders(`L1` and `L2`) proposing different membership(`m1` and `m2`(`mᵢ = cᵢcⱼ...`)):

`L1`: `m1`,
`L2`: `m2`

Thus the `L1` log history and the `L2` log history diverged.
Let `m0` be the last common membership in the log histories:

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
- `L1` must have committed log entry `m0` to a quorum in `m0`  in `term_1`.
- `L2` must have committed log entry `m0` to a quorum in `m0`, in `term_2`.

Assumes `term_1 < term_2`.

From (3) **initial-log**, `L2` has at least one log with `term_2` committed in a
quorum in `m0`.

∵ (2) **old-new-intersect** and `term_1 < term_2`

∴ log entry `m1` can never be committed by `L1`, 
  because log replication or voting will always see a higher `term_2` on a node in a quorum in `m0`.

  For the same reason, a candidate with log entry `m1` can never become a leader.

∴ It is impossible that there are two leaders that both can commit a log entry.

QED.
