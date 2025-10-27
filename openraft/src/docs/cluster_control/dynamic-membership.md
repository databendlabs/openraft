# Dynamic Membership

Openraft considers all memberships as **joint** memberships.
A uniform config is a special case: a joint config with only one config set.

## Membership APIs

### [`Raft::add_learner()`]

Adds a learner to the cluster and starts log replication from the leader.

**Parameters:**
- `node_id`: ID of the new node
- `node`: Node metadata (e.g., network address)
- `blocking`: If `true`, waits until the learner catches up with the leader's log

**Behavior:**
- Learner immediately receives log replication
- Learner does not participate in voting or elections
- If the node already exists as learner or voter, it is re-added with updated metadata

**Example:**
```ignore
// Non-blocking: returns after setting up replication
raft.add_learner(4, node, false).await?;

// Blocking: waits until caught up
raft.add_learner(4, node, true).await?;
```

### [`Raft::change_membership()`]

Changes the voting membership through a two-phase [joint consensus][`joint_consensus`] process.

**Parameters:**
- `members`: New voter set or node updates
- `retain`: If `true`, removed voters become learners; if `false`, they are removed entirely

**Preconditions:**
- Only the leader can change membership
- All nodes in `members` must already be learners (added via [`Raft::add_learner()`])

**Process:**
1. Proposes joint config: `[old_config, new_config]`
2. After joint config commits, proposes uniform config: `new_config`

**Behavior of `retain`:**

Given membership `{"voters":{1,2,3}, "learners":{}}`, calling `change_membership({3,4,5}, retain)`:
- If `retain=true`: Result is `{"voters":{3,4,5}, "learners":{1,2}}`
- If `retain=false`: Result is `{"voters":{3,4,5}, "learners":{}}`

**Example:**
```ignore
// Add learners first
raft.add_learner(2, node2, true).await?;
raft.add_learner(3, node3, true).await?;

// Promote to voters
raft.change_membership(btreeset!{1,2,3}, false).await?;
```

See [cluster example](https://github.com/databendlabs/openraft/blob/d041202a9f30b704116c324a6adc4f2ec28029fa/examples/raft-kv-memstore/tests/cluster/test_cluster.rs#L75-L103) for complete code.


## Updating Node Metadata

To update node metadata (e.g., network address), use `ChangeMembers::SetNodes`.

**⚠️ Warning:** Misusing `SetNodes` can cause split-brain. Use `RemoveNodes` + `add_learner` instead when possible.

### Split-Brain Risk

When updating node network addresses,
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


**Recommendation:** Use `RemoveNodes` + `add_learner` instead of `SetNodes` to avoid split-brain.

### Network Implementation Safety

[`RaftNetworkFactory`] and [`RaftNetworkV2`] implementations must ensure connections to the correct nodes.

Exercise additional care when:
- Nodes have conflicting metadata (e.g., duplicate hostnames)
- One node migrates to another's hostname
- Network cannot be trusted (adversary may reroute messages)



## See Also

- [`joint_consensus`]: Details on the two-phase membership change protocol
- [`node_lifecycle`]: Internal mechanics of node state transitions
- [`monitoring_maintenance`]: Operational guide for cluster monitoring and maintenance

[`Raft::add_learner()`]: `crate::Raft::add_learner`
[`Raft::change_membership()`]: `crate::Raft::change_membership`
[`ChangeMembers::SetNodes`]: `crate::change_members::ChangeMembers::SetNodes`
[`RaftNetworkFactory`]: `crate::network::RaftNetworkFactory`
[`RaftNetworkV2`]: `crate::network::v2::RaftNetworkV2`
[`joint_consensus`]: `crate::docs::cluster_control::joint_consensus`
[`node_lifecycle`]: `crate::docs::cluster_control::node_lifecycle`
[`monitoring_maintenance`]: `crate::docs::cluster_control::monitoring_maintenance`
