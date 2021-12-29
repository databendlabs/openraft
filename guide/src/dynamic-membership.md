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

If there are nodes in the given config is not a learner, this method will add it
as Learner first.
Thus it is recommended that application always call `Raft::add_learner` first.
Otherwise `change_membership` may block for long before committing the
given membership and return.

The new membership config specified by this method will take effect at once.

Once the new config is committed, a Voter that is no in the new config will
revert to a Learner and is ready to remove.

Correctness constrains:

- There is no uncommitted membership config on the leader.

- The new config has to contains a group that is the same as one of the last
    committed config.

    E.g., the last committed one is `[{a, b, c}]`, then a legal new config may be:
    a joint config: `[{a, b, c}, {x, y, z}]`.

    If the last committed one is `[{a, b, c}, {x, y, z}]`, a legal new config
    may be: `[{a, b, c}]`, or `[{x, y, z}]`.

If the constrains are not satisfied, an error is returned and nothing is done
except the learner will not be removed.

TODO(xp): these constrains can be loosen, if raft stores the commit index.
