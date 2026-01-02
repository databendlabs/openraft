### How to synchronize snapshots from leader to followers without rebuilding?

By default, each node builds its own snapshots independently. If you want the leader to build
a snapshot once and distribute it to followers (to save CPU/memory), you can implement this
using [`Raft::with_state_machine`][] and custom state machine methods.

**Why this isn't part of the core protocol:**

The Raft protocol only sends snapshots when a follower is missing logs (after log compaction).
In this case, the snapshot is always newer than the follower's state, so there's no need to
"save without installing". The edge case of receiving an old snapshot (e.g., delayed network
message) should simply be ignored.

**Key insight: Your state machine, your methods**

The [`RaftStateMachine`][] trait defines the minimum interface Openraft needs. Your
implementation can (and should) include additional methods for application-specific operations.

**Implementation approach:**

1. **Add custom methods to your state machine:**

```rust
pub struct MyStateMachine {
    // Your data
}

impl MyStateMachine {
    /// Custom method: force save snapshot without version checks
    /// This is YOUR method, not part of the RaftStateMachine trait
    pub async fn force_save_snapshot(&mut self, data: Vec<u8>) -> Result<(), io::Error> {
        // Your custom save logic - you control the rules
        self.snapshot_store.write(&data).await
    }
}

impl RaftStateMachine<TypeConfig> for MyStateMachine {
    // Standard trait methods...
}
```

2. **Use [`Raft::with_state_machine`][] to call your custom methods:**

```rust
// On follower: receive and save snapshot from leader
let snapshot_data = receive_snapshot_from_leader().await?;

raft.with_state_machine(|sm: &mut MyStateMachine| {
    Box::pin(async move {
        // Call YOUR custom method
        sm.force_save_snapshot(snapshot_data).await
    })
}).await??;
```

**Example: Custom snapshot distribution:**

```rust
// Leader builds and distributes snapshot
async fn distribute_snapshot(
    leader: &Raft<TypeConfig>,
    followers: Vec<NodeId>
) -> Result<(), io::Error> {
    // Build snapshot on leader
    let snapshot = leader.trigger().snapshot().await?
        .expect("snapshot built");

    // Send to followers via custom RPC
    for follower_id in followers {
        let snapshot_data = read_snapshot_file(&snapshot).await?;
        send_snapshot_rpc(follower_id, snapshot_data).await?;
    }
    Ok(())
}

// Follower receives and saves snapshot
async fn receive_snapshot(
    follower: &Raft<TypeConfig>,
    snapshot_data: Vec<u8>
) -> Result<(), io::Error> {
    follower.with_state_machine(|sm: &mut MyStateMachine| {
        Box::pin(async move {
            sm.save_snapshot_data(&snapshot_data).await
        })
    }).await?
}
```

**Note:** This optimization is application-specific. The core Raft protocol handles snapshot
transfer automatically when needed for log replication.

[`Raft::with_state_machine`]: `crate::Raft::with_state_machine`
[`RaftStateMachine`]: `crate::storage::RaftStateMachine`
