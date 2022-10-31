# Dynamic Membership

Unlike the original raft, openraft treats all membership as a **joint** membership.
A uniform config is just a special case of joint: the joint of only one config.

Openraft offers these mechanisms for controlling member node lifecycle:


## Membership API

### `Raft::add_learner(node_id, node, blocking)`

This method will add a learner to the cluster,
and immediately begin syncing logs from the leader.

- A **Learner** won't vote for leadership.


### `Raft::change_membership(members, allow_lagging, turn_to_learner)`

This method initiates a membership change and returns when the effective
membership becomes `members` and is committed.

If there are nodes in the given membership that is not a `Learner`, this method will fail.
Thus the application should always call `Raft::add_learner()` first.

Once the new membership is committed, a `Voter` not in the new config is removed if `turn_to_learner=false`,
and it is reverted to a `Learner` if `turn_to_learner=true`.


#### Example of `turn_to_learner`

Given the original membership to be `{"members":{1,2,3}, "learners":{}}`,
call `change_membership` with `members={3,4,5}`, then:

- If `turn_to_learner=true`,  the new membership is `{"members":{3,4,5}, "learners":{1,2}}`.
- If `turn_to_learner=false`, the new membership is `{"members":{3,4,5}, "learners":{}}`.


## Add a new node as a voter

To add a new node as a voter:
- First, add it as a `learner`(non-voter) with `Raft::add_learner()`.
  In this step, the leader sets up replication to the new node, but it can not vote yet.
- Then turn it into a `voter` with `Raft::change_membership()`.

```rust
let client = ExampleClient::new(1, get_addr(1)?);

client.add_learner((2, get_addr(2)?)).await?;
client.add_learner((3, get_addr(3)?)).await?;
client.change_membership(&btreeset! {1,2,3}).await?;
```

A complete snippet of adding voters can be found in [the example app](https://github.com/datafuselabs/openraft/blob/d041202a9f30b704116c324a6adc4f2ec28029fa/examples/raft-kv-memstore/tests/cluster/test_cluster.rs#L75-L103).


## Remove a voter node

-   Call `Raft::change_membership()` on the leader to initiate a two-phase
    membership config change, e.g., the leader will propose two config logs:
    joint config log: `[{1, 2, 3}, {3, 4, 5}]` and then the uniform config log:
    `{3, 4, 5}`.

-   As soon as the leader commits the second config log, the node to remove can
    be terminated safely.

**Note that** An application does not have to wait for the config log to be
replicated to the node to remove. Because a distributed consensus protocol
tolerates a minority member crash.


## Extended membership change algo

Openraft tries to commit one or more membership logs to finally change the
membership to `node_list`.
In every step, the log it tries to commit is:

-   the `node_list` itself, if it is safe to change from the previous membership to
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
