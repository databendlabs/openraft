Storage
=======
The way that data is stored and represented is an integral part of every data storage system. Whether it is a SQL or NoSQL database, a columner store, a KV store, or anything which stores data, control over the storage technology and technique is critical.

This implementation of Raft uses the `RaftStorage` trait to define the behavior needed of an application's storage layer to work with Raft. This is definitely the most complex looking trait in this crate. Ultimately the implementing type must be an Actix [`Actor`](https://docs.rs/actix/latest/actix) and it must implement handlers for a specific set of message types.

When creatinga new `RaftStorage` instance, it would be logical to supply the ID of the parent Raft node as well as the node's snapshot directory. Such information is needed when booting a node for the first time.

```rust
pub trait RaftStorage<D, R, E>: 'static
    where
        D: AppData,
        R: AppDataResponse,
        E: AppError,
{
    /// The type to use as the storage actor. Should just be Self.
    type Actor: Actor<Context=Self::Context> +
        Handler<GetInitialState<E>> +
        Handler<SaveHardState<E>> +
        Handler<GetLogEntries<D, E>> +
        Handler<AppendEntryToLog<D, E>> +
        Handler<ReplicateToLog<D, E>> +
        Handler<ApplyEntryToStateMachine<D, R, E>> +
        Handler<ReplicateToStateMachine<D, E>> +
        Handler<CreateSnapshot<E>> +
        Handler<InstallSnapshot<E>> +
        Handler<GetCurrentSnapshot<E>>;

    /// The type to use as the storage actor's context. Should be `Context<Self>` or `SyncContext<Self>`.
    type Context: ActorContext +
        ToEnvelope<Self::Actor, GetInitialState<E>> +
        ToEnvelope<Self::Actor, SaveHardState<E>> +
        ToEnvelope<Self::Actor, GetLogEntries<D, E>> +
        ToEnvelope<Self::Actor, AppendEntryToLog<D, E>> +
        ToEnvelope<Self::Actor, ReplicateToLog<D, E>> +
        ToEnvelope<Self::Actor, ApplyEntryToStateMachine<D, R, E>> +
        ToEnvelope<Self::Actor, ReplicateToStateMachine<D, E>> +
        ToEnvelope<Self::Actor, CreateSnapshot<E>> +
        ToEnvelope<Self::Actor, InstallSnapshot<E>> +
        ToEnvelope<Self::Actor, GetCurrentSnapshot<E>>;
}
```

Actix handlers must be implemented for the following types, all of which are found in the `storage` module:
- [GetInitialState](https://docs.rs/actix-raft/latest/actix_raft/storage/struct.GetInitialState.html): A request from Raft to get Raft's state information from storage.
- [SaveHardState](https://docs.rs/actix-raft/latest/actix_raft/storage/struct.SaveHardState.html): A request from Raft to save its HardState.
- [GetLogEntries](https://docs.rs/actix-raft/latest/actix_raft/storage/struct.GetLogEntries.html): A request from Raft to get a series of log entries from storage.
- [AppendEntryToLog](https://docs.rs/actix-raft/latest/actix_raft/storage/struct.AppendEntryToLog.html): A request from Raft to append a new entry to the log.
- [ReplicateToLog](https://docs.rs/actix-raft/latest/actix_raft/storage/struct.ReplicateToLog.html): A request from Raft to replicate a payload of entries to the log.
- [ApplyEntryToStateMachine](https://docs.rs/actix-raft/latest/actix_raft/storage/struct.ApplyEntryToStateMachine.html): A request from Raft to apply the given log entry to the state machine.
- [ReplicateToStateMachine](https://docs.rs/actix-raft/latest/actix_raft/storage/struct.ReplicateToStateMachine.html): A request from Raft to apply the given log entries to the state machine, as part of replication.
- [CreateSnapshot](https://docs.rs/actix-raft/latest/actix_raft/storage/struct.CreateSnapshot.html): A request from Raft to have a new snapshot created which covers the current breadth of the log.
- [InstallSnapshot](https://docs.rs/actix-raft/latest/actix_raft/storage/struct.InstallSnapshot.html): A request from Raft to have a new snapshot written to disk and installed.
- [GetCurrentSnapshot](https://docs.rs/actix-raft/latest/actix_raft/storage/struct.GetCurrentSnapshot.html): A request from Raft to get metadata of the current snapshot.

The following sections detail how to implement a safe and correct storage system for Raft using the `RaftStorage` trait. A very important note to keep in mind: data storage, data layout, data representation ... all of that is up to the implementor of the `RaftStorage` trait. That's the whole point. Every application is going to have nuances in terms of what they need to do at the storage layer. This is one of the primary locations where an application can innovate and differentiate.

### state
This pertains to implementing the `GetInitialState` & `SaveHardState` handlers.

##### `GetInitialState`
When the storage system comes online, it should check for any state currently on disk. Based on how the storage layer is persisting data, it may have to look in a few locations to get all of the needed data. Once the [`InitialState`](https://docs.rs/actix-raft/latest/actix_raft/storage/struct.InitialState.html) data has been collected, respond.

##### `SaveHardState`
This handler will be called periodically based on different events happening in Raft. Primarily, membership changes and elections will cause this to be called. Implementation is simple. Persist the data in the given [`HardState`](https://docs.rs/actix-raft/latest/actix_raft/storage/struct.HardState.html) to disk, ensure that it can be accurately retrieved even after a node failure, and respond.

### log & state machine
This pertains to implementing the `GetLogEntries`, `AppendEntryToLog`, `ReplicateToLog`, `ApplyEntryToStateMachine` & `ReplicateToStateMachine` handlers.

Traditionally, there are a few different terms used to refer to the log of mutations which are to be applied to a data storage system. Write-ahead log (WAL), op-log, there are a few different terms, sometimes with different nuances. In Raft, this is known simply as the log. A log entry describes the "type" of mutation to be applied to the state machine, and the state machine is the actual business-logic representation of all applied log entries.

##### `GetLogEntries`
This will be called at various times to fetch a range of entries from the log. The `start` field is inclusive, the `stop` field is non-inclusive. Simply fetch the specified range of logs from the storage medium, and return them.

##### `AppendEntryToLog`
Called as the direct result of a client request and will only be called on the Raft leader node. **THIS IS THE ONE AND ONLY** `RaftStorage` handler which is allowed to return errors which will not cause the Raft node to terminate. Reveiw the docs on the [`AppendEntryToLog`](https://docs.rs/actix-raft/latest/actix_raft/storage/struct.AppendEntryToLog.html) type, and you will see that its message response type is the `AppError` type, which is a statically known error type chosen by the implementor (which was reviewed earlier in the [raft overview chapter](https://railgun-rs.github.io/actix-raft/raft.html)).

This is where an application may enforce business-logic rules, such as unique indices, relational constraints, type validation, whatever is needed by the application. If everything checks out, insert the entry at its specified index in the log. **Don't just blindly append,** use the entry's index. There are times when log entries must be overwritten, and Raft guarantees the safety of such operations.

**Another very important note:** per the Raft spec in §8, to ensure that client requests are not applied > 1 due to a failure scenario and the client issuing a retry, the Raft spec recommends that applications track client IDs and use serial numbers on each request. This handler may then use that information to reject duplicate request using an application specific error. The application's client may observe this error and treat it as an overall success. This is an application level responsibility, Raft simply provides the mechanism to be able to implement it.

##### `ReplicateToLog`
This is similar to `AppendEntryToLog` except that this handler is only called on followers, and they should never perform validation or falible operations. If this handler returns an error, the Raft node will terminate in order to guard against data corruption. As mentioned previously, there are times when log entries must be overwritten. Raft guarantees the safety of these operations. **Use the index of each entry when inserting into the log.**

##### `ApplyEntryToStateMachine`
Once a log entry is known to be committed (it has been replicated to a majority of nodes in the cluster), the leader will call this handler to apply the entry to the application's state machine. Committed entries will never be removed or overwritten in the log, which is why it is safe to apply the entry to the state machine. To implement this handler, apply the contents of the entry to the application's state machine in whatever way is needed. This handler is allowed to return an application specific response type, which allows the application to return arbitrary information about the process of applying the entry.

For example, if building a SQL database, and the entry calls for inserting a new record and the full row of data needs to be returned to the client, this handler may return such data in its response.

Raft, as a protocol, guarantees strict linearizability. Entries will never be re-applied. The only case where data is removed from the state machine is during some cases of snapshotting where the entire state machine needs to be rebuilt. Read on for more details.

**NOTE WELL:** there are times when Raft needs to append blank entries to the log which will end up being applied to the state machine. See §8 for more details. Application's should handle this with a "no-op" variant of their `AppDataResponse` type.

##### `ReplicateToStateMachine`
This is similar to `ApplyEntryToStateMachine` except that this handler is only called on followers as part of replication, and are not allowed to return response data (as there is nothing to return response data to during replication).

### snapshots & log compaction
This pertains to implementing the `CreateSnapshot`, `InstallSnapshot` & `GetCurrentSnapshot`.

The snapshot and log compaction capabilities defined in the Raft spec are fully supported by this implementation. The storage layer is left to the application which uses this Raft implementation, but all snapshot behavior defined in the Raft spec is supported. Additionally, this implemention supports:

- Configurable snapshot policies. This allows nodes to perform log compacation at configurable intervals.
- Leader based `InstallSnapshot` RPC support. This allows the Raft leader to make determinations on when a new member (or a slow member) should receive a snapshot in order to come up-to-date faster.

For clarity, **it is emphasized** that implementing the log compaction & snapshot creation behavior is up to the `RaftStorage` implementor. This guide is here to help, and §7 of the Raft spec is dedicated to the subject.

##### `CreateSnapshot`
This handler is called when the Raft node determines that a snapshot is needed based on the cluster's configured snapshot policy. `Raft` guarantees that this interface will never be called multiple overlapping times, and it will not be called when an `InstallSnapshot` operation is in progress.

**It is critical to note** that the newly created snapshot must be able to be used to completely and accurately create a state machine. In addition to saving space on disk (log compaction), snapshots are used to bring new Raft nodes and slow Raft nodes up-to-speed with the cluster leader.

**implementation algorithm:**
- The generated snapshot should include all log entries starting from entry `0` up through the index specified by `CreateSnapshot.through`. This will include any snapshot which may already exist. If a snapshot does already exist, the new log compaction process should be able to just load the old snapshot first, and resume processing from its last entry.
- The newly generated snapshot should be written to the configured snapshot directory.
- All previous entries in the log should be deleted up to the entry specified at index `through`.
- The entry at index `through` should be replaced with a new entry created from calling [`Entry::new_snapshot_pointer(...)`](https://docs.rs/actix-raft/latest/actix_raft/messages/struct.Entry.html).
- Any old snapshot will no longer have representation in the log, and should be deleted.
- Return a [`CurrentSnapshotData`](https://docs.rs/actix-raft/latest/actix_raft/storage/struct.CurrentSnapshotData.html) struct which contains all metadata pertinent to the snapshot.

##### `InstallSnapshot`
This handler is called when the leader of the Raft cluster has determined that the subject node needs to receive a new snapshot. This is typically the case when new nodes are added to a running cluster, or if a node has gone offline for some amount of time without being removed from the cluster, or the node is VERY slow.

This message holds an `UnboundedReceiver` which will stream in new chunks of data as they are received from the Raft leader. See the docs on the [InstallSnapshotChunk](https://docs.rs/actix-raft/latest/actix_raft/storage/struct.InstallSnapshotChunk.html) for more info.

**implementation algorithm:**
- Upon receiving the request, a new snapshot file should be created on disk.
- Every new chunk of data received should be written to the new snapshot file starting at the `offset` specified in the chunk. Once the chunk has been successfully written, the `InstallSnapshotChunk.cb` (a `oneshot::Sender`) should be called to indicate that the storage engine has finished writing the chunk.
- If the receiver is dropped, the snapshot which was being created should be removed from disk, and a success response should be returned.

Once a chunk is received which is the final chunk of the snapshot (`InstallSnapshotChunk.done`), after writing the chunk's data, there are a few important steps to take:

- Create a new entry in the log via the [`Entry::new_snapshot_pointer(...)`](https://docs.rs/actix-raft/latest/actix_raft/messages/struct.Entry.html) constructor. Insert the new entry into the log at the specified `index` of the original `InstallSnapshot` payload.
- If there are any logs older than `index`, remove them.
- If there are any other snapshots in the configured snapshot dir, remove them.
- If an existing log entry has same index and term as snapshot's last included entry, retain log entries following it, then return.
- Else, discard the entire log leaving only the new snapshot pointer. **The state machine must be rebuilt from the new snapshot.** Return once the state machine has been brought up-to-date.

##### `GetCurrentSnapshot`
A request to get information on the current snapshot. `RaftStorage` implementations must take care to ensure that there is only ever one active snapshot, old snapshots should be deleted as part of `CreateSnapshot` and `InstallSnapshot` requests, and the snapshot information should be able to be retrieved efficiently. Having to load and parse the entire snapshot on each `GetCurrentSnapshot` request may not be such a great idea! Snapshots can be quite large.

----

Woot woot! Made it through the hard part! There is more to learn, so let's keep going.
