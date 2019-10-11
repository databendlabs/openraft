changelog
=========
### 0.4
#### 0.4.0
This changeset introduces a new `AppDataResponse` type which represents a concrete data type which must be sent back from the `RaftStorage` impl from the `ApplyEntryToStateMachine` handler. This provides a more direct path for returning application level data from the storage impl. Often times this is needed for responding to client requests in a timely / efficient manner.

- `AppDataResponse` type has been added (see above).
- A few handlers have been updated in the `RaftStorage` type. The handlers are now separated based on where they are invoked from the Raft node. The three changed handlers are:
  - `AppendEntryToLog`: this is the same. It is the initial step of handling client requests to apply an entry to the log. This is still where application level errors may be safely returned to the client.
  - `ReplicateToLog`: this is for replicating entries to the log. This is part of the replication process.
  - `ApplyEntryToStateMachine`: this is for applying an entry to the state machine as the final part of a client request. This is where the new `AddDataResponse` type must be returned.
  - `ReplicateToStateMachine`: this is for replicating entries to the state machine. This is part of the replication process.

### 0.3
#### 0.3.1
Overhauled the election timeout mechanism. This uses an interval job instead of juggling a rescheduling processes. Seems to offer quite a lot more stability. Along with the interval job, we are using std::time::Instants for performing the comparisons against the last received heartbeat.

#### 0.3.0
Another backwards incompatible change to the `RaftStorage` trait. It is now using associated types to better express the needed trait constraints. These changes were the final bit of work needed to get the entire actix-raft system to work with a Synchronous `RaftStorage` impl. Async impls continue to work as they have, the `RaftStorage` impl block will need to be updated to use the associated types though. The recommend pattern is as follows:

```rust
impl RaftStorage<..., ...> for MyStorage {
    type Actor = Self;
    type Context = Context<Self>; // Or SyncContext<Self>;
}
```

My hope is that this will be the last backwards incompatible change needed before a 1.0 release. This crate is still young though, so we will see.

### 0.2
#### 0.2.0
- Made a few backwards incompatible changes to the `RaftStorage` trait. Overwrite its third type parameter with `actix::SyncContext<Self>` to enable sync storage.
- Also removed the `RaftStorage::new` constructor, as it is a bit restrictive. Just added some docs instead describing what is needed.

### 0.1
#### 0.1.3
- Added a few addition top-level exports for convenience.

#### 0.1.2
- Changes to the README for docs.rs.

#### 0.1.1
- Changes to the README for docs.rs.

#### 0.1.0
- Initial release!
