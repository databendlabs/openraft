# Upgrade tips

First, have a look at the change log for the version to upgrade to.
A commit message starting with these keywords needs attention:

- Change: introduces breaking changes. Your application needs adjustment to pass compilation.
  If storage related data structure changed too, a data migration tool is required for the upgrade. See below.

- Feature: introduces non-breaking new features. Your application should compile without modification.

- Fix: bug fix. No modification is required.


## Upgrade from [v0.6.6](https://github.com/datafuselabs/openraft/tree/v0.6.6) to [v0.7.0](https://github.com/datafuselabs/openraft/tree/v0.7.0):

[Change log v0.7.0](https://github.com/datafuselabs/openraft/blob/release-0.7/change-log.md#v070)

To upgrade:

1. Update the application to adopt `v0.7.*` openraft.
  [`memstore`](https://github.com/datafuselabs/openraft/blob/release-0.7/memstore/src/lib.rs) is a good example about how to implement the openraft API.
  Also, the new implementation of `v0.7.*` has to pass [`RaftStorage` test suite](https://github.com/datafuselabs/openraft/blob/0355a6050e7cf6ecba83fe7c0f00abeeec3e3b15/memstore/src/test.rs#L26-L29).

2. Then shutdown all `v0.6.*` nodes and then bring up `v0.7.*` nodes.

  `v0.6.*` and `v0.7.*` should **NEVER** run in a same cluster, due to the data structure changes. 
  Exchanging data between `v0.6.*` and `v0.7.*` nodes may lead to data damage.

### API changes: 

- Removed methods:
  - `RaftStorage::get_membership_config()` is removed. It is provided with a default implementation in [`StorageHelper::get_membership()`](https://github.com/datafuselabs/openraft/blob/0355a6050e7cf6ecba83fe7c0f00abeeec3e3b15/openraft/src/storage_helper.rs#L70)
  - `RaftStorage::get_initial_state()` is removed too and is moved to `StorageHelper`.
  - `RaftStorage::get_log_entries()` is removed too and is moved to `StorageHelper`.
  - `RaftStorage::try_get_log_entry()` is removed too and is moved to `StorageHelper`.
  - `RaftStorage::first_id_in_log()` and `RaftStorage::last_id_in_log()` are removed. They are replaced with a single method: [`get_log_state()`](https://github.com/datafuselabs/openraft/blob/0355a6050e7cf6ecba83fe7c0f00abeeec3e3b15/openraft/src/types/v070/storage.rs#L47-L52), which returns both the first and the last log id in a `LogState` struct.
  - `RaftStorage::first_known_log_id()` is removed.
  
- Refactored methods:
  - `RaftStorage::last_applied_state()` return value changes from `(LogId, ...)` to `(Option<LogId>,...)`: i.e., `None` should be returned if the state machine has not yet applied any log.
  - `RaftStorage::delete_logs_from()` is split into two methods: [`delete_conflict_logs_since(log_id)`](https://github.com/datafuselabs/openraft/blob/0355a6050e7cf6ecba83fe7c0f00abeeec3e3b15/openraft/src/types/v070/storage.rs#L71) to delete logs that conflict with the leader on a **follower**, and [`purge_logs_upto(log_id)`](https://github.com/datafuselabs/openraft/blob/0355a6050e7cf6ecba83fe7c0f00abeeec3e3b15/openraft/src/types/v070/storage.rs#L74) to delete already applied logs. 
  
- Renamed methods: 
  - `RaftStorage::do_log_compaction()` is renamed to `build_snapstho()`, without any change.
  - `RaftStorage::finalize_snapshot_installation()` is renamed to `install_snapshot()`, without any change.


### Data changes:

Some log id fields are changed from `LogId` to `Option<LogId>`,
in order to correctly describe the initial states.
Now the `RaftStorage` implementation can just return `None` if the data to access is absent.
No need to build a valid default value any more.

- `InitialState.last_log_id: LogId` changes to `Option<LogId>`.
- `InitialState.last_applied: LogId` changes to `Option<LogId>`.
- `InitialState.last_membership: EffectiveMembership` changes to `Option<EffectiveMembership>`.
- `VoetRequest.last_log_id: LogId` changes to `Option<LogId>`.
- `VoetResponse.last_log_id: LogId` changes to `Option<LogId>`.
- `AppendEntriesRequest.last_log_id: LogId` changes to `Option<LogId>`.
- `AppendEntriesRequest.leader_commit: LogId` changes to `Option<LogId>`.
- `AddLearnerResponse.matched: LogId` changes to `Option<LogId>`

Other changes:

- `AppendEntriesResponse` now only needs to return `success: bool` and `conflict: bool`, no need to return the last log that matches, previously `AppendEntriesResponse.matched` or the log id that conflicts with the leader, previously `AppendEntriesResponse.conflig`.
- `ClientWriteRequest.entry` is renamed to `payload`.
- Add a new struct `LogState` as a return value of [`get_log_state()`](https://github.com/datafuselabs/openraft/blob/0355a6050e7cf6ecba83fe7c0f00abeeec3e3b15/openraft/src/types/v070/storage.rs#L47-L52).

  
## Upgrade from [v0.6.5](https://github.com/datafuselabs/openraft/tree/v0.6.5) to [v0.6.6](https://github.com/datafuselabs/openraft/tree/v0.6.6):

[Change log v0.6.6](https://github.com/datafuselabs/openraft/blob/release-0.6/change-log.md#v066)

just modify application code to pass compile.

- API changes: struct fields changed in `StorageIOError` and `Violation`.
- Data changes: none.

