# Guide for upgrading from [v0.6](https://github.com/datafuselabs/openraft/tree/v0.6.8) to [v0.7](https://github.com/datafuselabs/openraft/tree/v0.7.0):

[Change log v0.7.0](https://github.com/datafuselabs/openraft/blob/release-0.7/change-log.md#v070)

In this chapter, for users who will upgrade openraft 0.6 to openraft 0.7,
we are going to explain what changes has been made from openraft-0.6 to
openraft-0.7 and why these changes are made.

- A correct upgrading should compile and at least passes the [Storage test](https://github.com/datafuselabs/openraft/blob/release-0.7/memstore/src/test.rs), as `memstore` does.

- Backup your data before deploying upgraded application.

To upgrade:

1. Update the application to adopt `v0.7.*` openraft.
   [`memstore`](https://github.com/datafuselabs/openraft/blob/release-0.7/memstore/src/lib.rs) is a good example about how to implement the openraft API.
   Also, the new implementation of `v0.7.*` has to pass [`RaftStorage` test suite](https://github.com/datafuselabs/openraft/blob/0355a6050e7cf6ecba83fe7c0f00abeeec3e3b15/memstore/src/test.rs#L26-L29).

2. Then shutdown all `v0.6.*` nodes and then bring up `v0.7.*` nodes.

`v0.6.*` and `v0.7.*` should **NEVER** run in a same cluster, due to the data structure changes.
Exchanging data between `v0.6.*` and `v0.7.*` nodes may lead to data damage.


# Storage API changes

`RaftStorage`:

- The following APIs are removed from `RaftStorage`:

    ```ignore
    RaftStorage::get_membership_config();
    RaftStorage::get_initial_state();
    RaftStorage::get_log_entries();
    RaftStorage::try_get_log_entry();
    ```

  When migrating from 0.6 to 0.7, just remove implementations for these
  method. These methods are provided by a `StorageHelper` in 0.7 .

  **Why**:

  These tasks are almost the same in every openraft application.
  They should not bother application author to implement them.



- Merged the following
  [log state methods in openraft-0.6](https://github.com/datafuselabs/openraft/blob/ad2edf28232510aed79fad5b1dc8778a019fef2d/memstore/src/lib.rs#L298-L319) into one method [`RaftStorage::get_log_state()`](https://github.com/datafuselabs/openraft/blob/9b65015f55c5fe3c2a16b48a23d8d1a6c01787af/memstore/src/lib.rs#L161-L176):
    ```ignore
    RaftStorage::first_id_in_log();
    RaftStorage::first_known_log_id();
    RaftStorage::last_id_in_log();
    ```

  The new and only log state API signature in 0.7 is: `RaftStorage::get_log_state() -> Result<LogState, _>`.

  When migrating from 0.6 to 0.7, replace these three methods with `get_log_state()`.
  `get_log_state()` should returns the last purged log id and the last known
  log id. An application using openraft-0.7 should store the last-purged log id in its store,
  when [`RaftStorage::purge_logs_upto()`](https://github.com/datafuselabs/openraft/blob/9b65015f55c5fe3c2a16b48a23d8d1a6c01787af/memstore/src/lib.rs#L200-L219) is called.

  **Why**:

  Reading log state should be atomic, such a task should be done with
  one method.


- Split [`RaftStorage::delete_logs_from(since_log_index..upto_log_index)`](https://github.com/datafuselabs/openraft/blob/ad2edf28232510aed79fad5b1dc8778a019fef2d/memstore/src/lib.rs#L327-L343) into two methods:

    - [`RaftStorage::purge_logs_upto(log_id)`](https://github.com/datafuselabs/openraft/blob/9b65015f55c5fe3c2a16b48a23d8d1a6c01787af/memstore/src/lib.rs#L200-L219)
      Delete applied logs from the **beginning** to the specified log id and store
      the `log_id` in the store.

    - [`RaftStorage::delete_conflict_logs_since(log_id)`](https://github.com/datafuselabs/openraft/blob/9b65015f55c5fe3c2a16b48a23d8d1a6c01787af/memstore/src/lib.rs#L184-L197)
      Delete logs that conflict with the leader on a follower, since the
      specified `log_id` to the **last** log.

  **Why**:

  These two deleting logs tasks are slightly different:
    - Purging applied logs does not have to be done at once, it can be delayed by an application for performance concern.
    - Deleting conflicting logs has to be done before returning.

  And openraft does not allows a **hole** in logs, splitting `delete`
  operation into these two methods prevents punching a hole in logs,
  which is potentially a bug.


- The return value of `RaftStorage::last_applied_state()` is changed to `(Option<LogId>, _)`,
  since most `LogId` in openraft code base are replaced with `Option<LogId>`.

- Rename:

    ```ignore
    RaftStorage::do_log_compaction()              => build_snapshot()
    RaftStorage::finalize_snapshot_installation() => install_snapshot()
    ```

  There is no function changes with these two methods.



# Data type changes

- Replace several field type `LogId` with `Option<LogId>`.

  Storage related fields that are changed:

    ```ignore
    InitialState.last_log_id
    InitialState.last_applied
    SnapshotMeta.last_log_id
    ```

  RPC related fields that are changed:

    ```ignore
    VoteRequest.last_log_id
    VoteResponse.last_log_id
    AppendEntriesRequest.prev_log_id
    AppendEntriesRequest.leader_commit
    AppendEntriesResponse.conflict
    AddLearnerResponse.matched
    StateMachineChanges.last_applied
    ```

    When migrating from 0.6 to 0.7, wrap these fields with a `Some()`.

    **Why**:

    Explicitly describe uninitialized state by giving uninitialized state a
    different type variant.
    E.g., Using `0` as uninitialized log index, there is chance mistakenly using
    `0` to write to or read from the storage.


- For the similar reason, replace `EffectiveMembership` with
  `Option<EffectiveMembership>`.


- New struct `LogState`: `(LogState.last_purge_log_id, LogState.last_log_id]` is
  the range of all present logs in storage(left-open, right-close range).

  This struct type is introduced along with [`RaftStorage::get_log_state()`](https://github.com/datafuselabs/openraft/blob/9b65015f55c5fe3c2a16b48a23d8d1a6c01787af/memstore/src/lib.rs#L161-L176).


- RPC:
  `AppendEntriesResponse`: removed field `matched`: because if it is a
  successful response, the leader knows what the last matched log id.

  Add a new field `success` and change `conflict` to a simple `bool`, because
  if there is a log that conflicts with the leader's, it has to be the
  `prev_log_id`.

- `ClientWriteRequest`: removed it is barely a wrapper of EntryPayload
  client_write

