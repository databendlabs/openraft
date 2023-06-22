# Dynamic Membership

In contrast to the original raft, openraft considers all memberships as **joint** memberships.
A uniform config is simply a specific case of joint: the combination of only one config.

Openraft provides the following mechanisms for managing member node lifecycles:


## Membership API

### [`Raft::add_learner(node_id, node, blocking)`][`Raft::add_learner()`]

This method adds a learner to the cluster,
and immediately starts synchronizing logs from the leader.

- A **Learner** does not vote for leadership.


### [`Raft::change_membership(members, retain)`][`Raft::change_membership()`]

This method initiates a membership change and returns when the proposed
membership is effective and committed.

If there are nodes in the given membership that are not `Learners`, this method will fail.
Therefore, the application should always call [`Raft::add_learner()`] first.

Once the new membership is committed, a `Voter` not in the new config is removed if `retain=false`,
otherwise, it is converted to a `Learner` if `retain=true`.


#### Example of using `retain`

Given the original membership as `{"members":{1,2,3}, "learners":{}}`,
call `change_membership` with `members={3,4,5}`, then:

- If `retain=true`,  the new membership is `{"members":{3,4,5}, "learners":{1,2}}`.
- If `retain=false`, the new membership is `{"members":{3,4,5}, "learners":{}}`.


## Add a new node as a `Voter`

To add a new node as a `Voter`:
- First, add it as a `Learner`(non-voter) with [`Raft::add_learner()`].
  In this step, the leader sets up replication to the new node, but it cannot vote yet.
- Then, convert it into a `Voter` with [`Raft::change_membership()`].

```ignore
let client = ExampleClient::new(1, get_addr(1)?);

client.add_learner((2, get_addr(2)?)).await?;
client.add_learner((3, get_addr(3)?)).await?;
client.change_membership(&btreeset! {1,2,3}, true).await?;
```

A complete snippet of adding voters can be found in [Mem KV cluster example](https://github.com/datafuselabs/openraft/blob/d041202a9f30b704116c324a6adc4f2ec28029fa/examples/raft-kv-memstore/tests/cluster/test_cluster.rs#L75-L103).


## Remove a voter node

-   Call `Raft::change_membership()` on the leader to initiate a two-phase
    membership config change, e.g., the leader will propose two config logs:
    joint config log: `[{1, 2, 3}, {3, 4, 5}]` and then the uniform config log:
    `{3, 4, 5}`.

-   As soon as the leader commits the second config log, the node to remove can
    be safely terminated.

**Note that** An application does not have to wait for the config log to be
replicated to the node to remove. Because a distributed consensus protocol
tolerates a minority member crash.


To read more about Openraft's [Extended Membership Algorithm][`extended_membership`].


## Update Node

To update a node, such as altering its network address,
the application calls [`Raft::change_membership()`][].
The initial argument should be set to [`ChangeMembers::SetNodes(BTreeMap<NodeId,Node>)`][`ChangeMembers::SetNodes`].

**Warning: Misusing `SetNodes` could lead to a split-brain situation**:

### Brain split

When Updating node network addresses,
brain split could occur if the new address belongs to another node,
leading to two elected leaders.

Consider a 3-node cluster (`a, b, c`, with addresses `x, y, z`) and an
uninitialized node `d` with address `w`:

```text
a: x
b: y
c: z

d: w
```

Mistakenly updating `b`'s address from `y` to `w` would enable both `x, y` and `z, w` to form quorums and elect leaders:

- `c` proposes ChangeMembership: `{a:x, b:w, c:z}`;
- `c, d` grant `c`;

- `c` elects itself as leader
- `c, d` confirm `c` as leader

- `a` elects itself as leader
- `a, b` confirm `a` as leader


Directly updating node addresses with `ChangeMembers::SetNodes`
should be replaced with `ChangeMembers::RemoveNodes` and `ChangeMembers::RemoveNodes` whenever possible.

Do not use `ChangeMembers::SetNodes` unless you know what you are doing.


[`ChangeMembers::SetNodes`]: `crate::change_members::ChangeMembers::SetNodes`
[`Raft::add_learner()`]: `crate::Raft::add_learner`
[`Raft::change_membership()`]: `crate::Raft::change_membership`
[`extended_membership`]: `crate::docs::data::extended_membership`
