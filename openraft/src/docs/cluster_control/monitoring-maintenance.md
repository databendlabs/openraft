# Monitoring and Maintenance

This guide explains how to monitor a Raft cluster's health and perform maintenance operations like adding or removing nodes.

## Monitoring Cluster Health

### Accessing Metrics

Use [`Raft::metrics()`] to monitor cluster health. It returns a watch channel with real-time [`RaftMetrics`]:

```ignore
let metrics_rx = raft.metrics();

// Read current state
let metrics = metrics_rx.borrow_watched();
println!("Node state: {:?}", metrics.state);
println!("Current leader: {:?}", metrics.current_leader);
```

### Key Health Indicators

**Node State** ([`RaftMetrics::state`]): Whether the node is [`Leader`], [`Follower`], [`Learner`], or [`Candidate`].

**Current Leader** ([`RaftMetrics::current_leader`]): ID of the current leader, if known.

**Log State**:
- [`RaftMetrics::last_log_index`]: Last appended log index
- [`RaftMetrics::last_applied`]: Last applied log to state machine
- [`RaftMetrics::snapshot`]: Last log in snapshot

**Leader Health** ([`RaftMetrics::last_quorum_acked`]): For leaders only, timestamp of the last quorum acknowledgment. An old timestamp suggests the leader may be partitioned from the cluster.

[`Leader`]: `crate::core::ServerState::Leader`
[`Follower`]: `crate::core::ServerState::Follower`
[`Learner`]: `crate::core::ServerState::Learner`
[`Candidate`]: `crate::core::ServerState::Candidate`

### Detecting Node Failures

When this node is the leader, use these metrics to detect follower/learner issues:

**Heartbeat Metrics** ([`RaftMetrics::heartbeat`]): Maps each node ID to the last heartbeat acknowledgment time. Calculate the elapsed time to detect potentially offline nodes:

```ignore
if let Some(heartbeat) = &metrics.heartbeat {
    for (node_id, last_ack) in heartbeat {
        if let Some(ack_time) = last_ack {
            let elapsed = ack_time.elapsed();
            if elapsed > threshold {
                // Node may be offline or unreachable
            }
        }
    }
}
```

**Replication Metrics** ([`RaftMetrics::replication`]): Maps each node ID to the last matched log index. Compare with the leader's log to detect lagging nodes:

```ignore
if let Some(replication) = &metrics.replication {
    for (node_id, matched_log) in replication {
        if let Some(matched) = matched_log {
            let lag = metrics.last_log_index.unwrap_or(0) - matched.index;
            if lag > threshold {
                // Node is lagging behind
            }
        }
    }
}
```

## Maintenance Operations

When monitoring detects issues (offline nodes, excessive lag), perform maintenance operations.

See [`dynamic_membership`] for API details on `add_learner` and `change_membership`.

### When to Add Nodes

Add nodes when:
- Expanding cluster capacity
- Replacing failed nodes
- Increasing replication factor

**Process:**
1. Use [`Raft::add_learner()`] with `blocking=true` to add and wait for catch-up
2. Use [`Raft::change_membership()`] to promote to voter

### When to Remove Nodes

Remove nodes when:
- Decommissioning servers
- Reducing cluster size
- Node is permanently failed

**Process:**
1. Build new voter set excluding the failed node
2. Use [`Raft::change_membership()`] where:
   - `retain=true`: Node becomes learner (can be re-promoted later)
   - `retain=false`: Node removed completely from cluster

**Important:** Node can be terminated after the uniform config is committed (after the two-phase change completes).

## Automated Maintenance Cautions

When building automated maintenance systems, follow these safety guidelines:

### 1. Leader Uncertainty

A leader cannot definitively determine if a node has failed. The issue could be:
- Network partition affecting the follower
- Network issues with the leader itself
- Temporary connectivity problems

Don't immediately remove unresponsive nodes. Use multiple checks over time.

### 2. Leader Validity

The leader might be stale. Another node could be the current leader with a higher term. The stale leader's maintenance requests will be rejected by the actual leader.

Always check [`RaftMetrics::running_state`] for errors and verify leadership before operations.

### 3. Maintain Quorum

Removing nodes one by one without adding replacements can eventually leave a single-node cluster, affecting availability.

**Best practice**: Add a replacement node before removing a failed one.

### 4. Only Leaders Perform Maintenance

Only the cluster leader can perform membership changes. Followers must forward requests to the leader or wait until they become leader.

Check `metrics.state == `[`ServerState::Leader`] before attempting maintenance operations.

[`ServerState::Leader`]: `crate::core::ServerState::Leader`

## Example: Automated Node Replacement

```ignore
use std::time::Duration;

const OFFLINE_THRESHOLD: Duration = Duration::from_secs(30);
const MAX_LAG: u64 = 1000;

async fn check_and_maintain(raft: &Raft<TypeConfig>) -> Result<()> {
    let metrics = raft.metrics().borrow_watched();

    // Only leader performs maintenance
    if metrics.state != ServerState::Leader {
        return Ok(());
    }

    // Check for offline or lagging nodes
    if let (Some(heartbeat), Some(replication)) = (&metrics.heartbeat, &metrics.replication) {
        for (node_id, last_ack) in heartbeat {
            // Skip if heartbeat is recent
            if let Some(ack_time) = last_ack {
                if ack_time.elapsed() < OFFLINE_THRESHOLD {
                    continue;
                }
            }

            // Node appears offline - prepare replacement
            println!("Node {} appears offline", node_id);

            // 1. Add replacement node as learner
            let new_node = get_replacement_node().await?;
            raft.add_learner(new_node.id, new_node.addr, true).await?;

            // 2. Build new membership without failed node, including replacement
            let mut new_voters = metrics.membership_config
                .voter_ids()
                .filter(|id| id != node_id)
                .collect::<BTreeSet<_>>();
            new_voters.insert(new_node.id);

            // 3. Change membership
            raft.change_membership(new_voters, false).await?;

            break; // Handle one node at a time
        }
    }

    Ok(())
}
```

## Monitoring Best Practices

1. **Export metrics** to monitoring systems (Prometheus, etc.) for historical analysis
2. **Set up alerts** for leader changes, node failures, and replication lag
3. **Monitor leader lease** via `last_quorum_acked` to detect leader isolation
4. **Track membership changes** to audit cluster topology changes
5. **Use multiple indicators** before declaring a node failed

## See Also

- [`RaftMetrics`]: Complete metrics structure
- [`dynamic_membership`]: API details for `add_learner` and `change_membership`
- [`node_lifecycle`]: Internal mechanics of node state transitions

[`Raft::metrics()`]: `crate::Raft::metrics`
[`Raft::add_learner()`]: `crate::Raft::add_learner`
[`Raft::change_membership()`]: `crate::Raft::change_membership`
[`RaftMetrics`]: `crate::metrics::RaftMetrics`
[`RaftMetrics::state`]: `crate::metrics::RaftMetrics::state`
[`RaftMetrics::current_leader`]: `crate::metrics::RaftMetrics::current_leader`
[`RaftMetrics::last_log_index`]: `crate::metrics::RaftMetrics::last_log_index`
[`RaftMetrics::last_applied`]: `crate::metrics::RaftMetrics::last_applied`
[`RaftMetrics::snapshot`]: `crate::metrics::RaftMetrics::snapshot`
[`RaftMetrics::last_quorum_acked`]: `crate::metrics::RaftMetrics::last_quorum_acked`
[`RaftMetrics::heartbeat`]: `crate::metrics::RaftMetrics::heartbeat`
[`RaftMetrics::replication`]: `crate::metrics::RaftMetrics::replication`
[`RaftMetrics::running_state`]: `crate::metrics::RaftMetrics::running_state`
[`dynamic_membership`]: `crate::docs::cluster_control::dynamic_membership`
[`node_lifecycle`]: `crate::docs::cluster_control::node_lifecycle`
