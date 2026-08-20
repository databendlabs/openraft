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

### Removing a retained learner

`change_membership(..., retain=true)` only demotes a voter to a learner; the
demoted node stays in the membership and continues to receive log replication.
To fully evict it, follow up with [`ChangeMembers::RemoveNodes`] once you no
longer need the node hot.

**Example:**
```ignore
// Step 1: demote voter 1 to learner; cluster is now {voters:{2,3}, learners:{1}}
raft.change_membership(btreeset!{2,3}, true).await?;

// Step 2: fully evict learner 1; cluster is now {voters:{2,3}, learners:{}}
raft.change_membership(ChangeMembers::RemoveNodes(btreeset!{1}), false).await?;
```

Use the two-step sequence when you want a graceful drain — the demoted node
keeps replicating logs, so it stays warm and can be re-promoted quickly if you
need to roll back. For permanent removal in a single call, prefer
`change_membership(new_voters, false)` directly, which drops the removed voter
from the cluster without an intermediate learner state.


## Node IDs Must Not Be Reused

A node ID identifies one node, holding one log, for the entire lifetime of the
cluster. Once a node is removed, never assign its ID to a node that starts from
a different log, typically an empty one. Restarting a node with its data intact
is not reuse: the ID still designates the same log.

Reusing an ID is equivalent to letting that node revert its log, and it can
split the cluster into two independent quorums. The reason is that a node
replaying the log from an empty state passes through every **historical**
membership config on its way to the current one, and a membership config takes
effect as soon as it is appended, without waiting to be committed. A historical
config that contains the reused ID therefore becomes the effective membership
of the fresh node for a while.

Consider a cluster whose membership was once the single node `{n3}` and is now
`{n1, n2, n3}` with `n1` as the leader:

1. `n1` calls `change_membership({n1, n2})`, removing `n3`.
2. `n3` observes its removal and wipes its disk.
3. `n1` calls `add_learner(n3, ..)` and `change_membership({n1, n2, n3})`,
   giving the ID `n3` to the wiped node.
4. The wiped `n3` replays the log from the beginning. Partway through, it
   appends the historical membership entry `{n3}` and adopts it as its
   effective membership, in which `n3` alone is a quorum.
5. If replication to `n3` stalls before it catches up, because the leader
   crashed or the network partitioned, `n3`'s election timer fires. It elects
   itself with its own vote under config `{n3}` and commits writes, while
   `{n1, n2}` is still a quorum under the current config and commits writes of
   its own. The two quorums are split-brained.

```text
membership: {n3}      {n1,n2,n3}   {n1,n2}   {n1,n2,n3}
n1        |  ...  ------------------------------------->  quorum {n1,n2}
n2        |  ...  ------------------------------------->
n3        |  ...  --------------------------X  erased
n3(new)   |                                    {n3} -->   quorum {n3}
----------+------------------------------------------------------> time
```

Openraft cannot detect this. Removing a node discards the leader's replication
progress for it, so the empty log of the re-added `n3` does not look like a
[log reversion][`Config::allow_log_reversion`]; the caller must guarantee ID
uniqueness.

**Recommendation:** allocate a fresh, never-before-used ID for every node that
joins the cluster. A good way to do so is to let the cluster's own log allocate
it: when a node joins, the leader proposes a blank log entry and uses the index
of that entry as the new node's ID.

```ignore
// `Request::blank()` is an application request that changes nothing.
let resp = raft.client_write(Request::blank()).await?;
let node_id = resp.log_id.index();

raft.add_learner(node_id, node, true).await?;
```

Log indices strictly increase and no committed index is ever assigned twice, so
every ID obtained this way is one that no node has ever held, and no external ID
service is needed. If the blank write fails, no node has taken that index yet,
so retrying is safe.


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
[`ChangeMembers::RemoveNodes`]: `crate::change_members::ChangeMembers::RemoveNodes`
[`Config::allow_log_reversion`]: `crate::config::Config::allow_log_reversion`
[`RaftNetworkFactory`]: `crate::network::RaftNetworkFactory`
[`RaftNetworkV2`]: `crate::network::RaftNetworkV2`
[`joint_consensus`]: `crate::docs::cluster_control::joint_consensus`
[`node_lifecycle`]: `crate::docs::cluster_control::node_lifecycle`
[`monitoring_maintenance`]: `crate::docs::cluster_control::monitoring_maintenance`
