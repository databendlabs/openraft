### How to detect if a leader is valid?

**Problem**: In distributed systems, you can perceive multiple leaders at the
same time. However, only one of them is the **legal leader** at any specific
point in time - only one can actually commit logs successfully.

This commonly happens when:
- A leader node restarts and immediately resumes as Leader (by design)
- Network partitions occur
- There are message delays or concurrent elections

**Solution**: Use the [`RaftMetrics::last_quorum_acked`][] field to verify leadership validity.

The `last_quorum_acked` field shows the most recently acknowledged timestamp by a quorum:

```rust,ignore
/// For a leader, it is the most recently acknowledged timestamp by a quorum.
///
/// It is `None` if this node is not leader, or the leader is not yet acknowledged by a quorum.
/// Being acknowledged means receiving a reply of
/// `AppendEntries`(`AppendEntriesRequest.vote.committed == true`).
pub last_quorum_acked: Option<SerdeInstantOf<C>>,
```

**A valid leader must have `last_quorum_acked` as `Some` with a recent timestamp:**

- **`None`**: Invalid leader (not acknowledged by quorum)
- **`Some(old_timestamp)`**: Invalid leader (lost connection with cluster)
- **`Some(recent_timestamp)`**: Valid leader

**Example**:

```rust,ignore
let mut rx = self.raft.metrics();
loop {
    let m = rx.borrow();

    // Only act if:
    // 1. State shows Leader, AND
    // 2. last_quorum_acked is Some with a recent timestamp
    if m.state == ServerState::Leader {
        if let Some(last_acked) = m.last_quorum_acked {
            // Check if acknowledged recently (e.g., within 2× election timeout)
            let election_timeout = Duration::from_millis(500); // your configured timeout
            if last_acked.elapsed() < election_timeout * 2 {
                do_critical_leader_operation();
            }
        }
        // If last_quorum_acked is None, this is NOT a valid leader
    }

    rx.changed().await?;
}
```

If you want to be extra careful, wait for metrics to flush by adding a small
delay and checking again. However, **`None` itself means the leader is not
valid**.

[`RaftMetrics::last_quorum_acked`]: `crate::metrics::RaftMetrics::last_quorum_acked`
