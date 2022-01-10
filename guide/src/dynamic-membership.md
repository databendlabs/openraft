# Dynamic Membership

This impl of `Raft` offers a more flexible way to deal with cluster management:

It offers only one cluster membership: the **joint** membership.
**Joint** membership is no more an ephemeral state but instead, a normal
long lasting config for a cluster.

A uniform config is just a special case.

> This way it offers flexible cluster management such as hierarchical
> membership config that is aware of data-center layout, e.g., 
> `[{a, b, c}, {u, v, w}, {x, y, z}]` is a cluster with 9 nodes, every 3 nodes
> forms a group(which in real world, a data center.).
> And the quorum of such a cluster is majority of 3 groups, in which
> a majority is constituted, e.g, `[{a, b}, {u, v}]` is a quorum.

---

Openraft offers 2 mechanisms for controlling member node lifecycle.


## `Raft::add_learner()`

This method will add a learner to the cluster,
and immediately begin syncing logs from the leader.

- A **Learner** won't vote for leadership.

- A **Learner** is not persistently stored by `Raft`, i.e., if a new leader is
    elected, a Learner will no longer receive logs from the new leader.

    TODO(xp): store learners in `MembershipConfig`.


## `Raft::change_membership()`

This method will initiate a membership change.

If there are nodes in the given membership that is not a learner, this method will add it
as Learner first.
Thus it is recommended that application always call `Raft::add_learner` first.
Otherwise `change_membership` may block for long before committing the
given membership and return.

The new membership specified by this method will take effect at once.

Once the new membership is committed, a Voter that is not in the new membership will
revert to a Learner and is ready to remove.

Correctness constrains:

- There is no uncommitted membership on the leader.

- The new membership has to contains a config that is the same as one of the last
    committed membership.

    E.g., the last committed one is `[{a, b, c}]`, then a legal new membership may be:
    a joint membership: `[{a, b, c}, {x, y, z}]`.

    If the last committed one is `[{a, b, c}, {x, y, z}]`, a legal new
    membership may be: `[{a, b, c}]`, or `[{x, y, z}]`.

If the constraints are not satisfied, an error is returned and nothing is done
except the learner will not be removed.


### The safe-to relation

TL, DR

According to the rule that every 2 quorums has to intersect each other,
one membership `m1` is safe to change to another membership `m2` iff `m1` contains
a config the same as one in `m2`.

E.g.: given a joint membership of 3 config: `m1 = c1 * c2 * c3`(`cᵢ` is a set of
nodes. `*` means joint), the following are safe with `m1`:
- `c1`,
- `c2`,
- `c1 * c3`,
- and `c2 * c4 * c5`.


The `safe_to` relation is **commutative**, e.g., `m1` being safe with `m2` iff `m2` being safe with `m1`.

E.g., one of the safe membership changing path is:
`c1`  →  `c1 * c2 * c3`  →  `c3 * c4`  →  `c4`.
Every step in it is safe.

Another example with 6 memberships shows that it is always safe to change
membership from one vertex to another along edges:

```
            c3
          /   \
         /     \
        /       \
   c1*c3 ------- c2*c3
    /  \         /  \
   /    \       /    \
  /      \     /      \
c1 ------ c1*c2 ------ c2
```
