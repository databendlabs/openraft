## v0.8.4

Summary:

- Added:
    -   [e39da9f0](https://github.com/datafuselabs/openraft/commit/e39da9f022f5411b8828fac096cd8bcd118d6426) define custom `Entry` type for raft log.
    -   [87d62d56](https://github.com/datafuselabs/openraft/commit/87d62d5666af0a0d7f7ffa056057f53f1402d33e) add feature flag storage-v2 to enable `RaftLogStorage` and `RaftStateMachine`.
    -   [229f3368](https://github.com/datafuselabs/openraft/commit/229f33689fd949eca9fa29fadf2e1285fcb21365) add backoff strategy for unreachable nodes.
    -   [f0dc0eb7](https://github.com/datafuselabs/openraft/commit/f0dc0eb7f7fa18d5f546615c51c0f74815457449) add `RaftMetrics::purged` to report the last purged log id.
    -   [37e96482](https://github.com/datafuselabs/openraft/commit/37e96482e4f446586af55827cbfff51102f2058e) add `Wait::purged()` to wait for purged to become the expected.
    -   [0b419eb6](https://github.com/datafuselabs/openraft/commit/0b419eb619b15139df50798deca61e61998e4c41) add `RaftMetrics.vote`, `Wait::vote()`.
    -   [ee7b8853](https://github.com/datafuselabs/openraft/commit/ee7b8853d935ce5f47848d8f62b09e5c83c05c69) new `RaftNetwork` API with argument `RCPOption`.
    -   [1ee82cb8](https://github.com/datafuselabs/openraft/commit/1ee82cb82ed9a33015e23fe4ba571a42323cc7cd) `RaftNetwork::send_append_entries()` can return `PartialSuccess`.
    -   [e9eed210](https://github.com/datafuselabs/openraft/commit/e9eed210fc2977c9e6e94a6fc1dd2cadf51362e7) leader lease.
    -   [269d221c](https://github.com/datafuselabs/openraft/commit/269d221c0fd43cab4a4064de342943e58adf2757) add `SnapshotPolicy::Never`.
    -   [9e7195a1](https://github.com/datafuselabs/openraft/commit/9e7195a1f37ff09f45a4f97f2b6a0473c322c604) add `Raft::purge_log()`.
- Improved:
    -   [dbac91d5](https://github.com/datafuselabs/openraft/commit/dbac91d5dc26fd92a8ac5038220176823d9d6d29) send `AppendEntries` response before committing entries.
    -   [6769cdd0](https://github.com/datafuselabs/openraft/commit/6769cdd09c045757b19cbb3222225b86db28e936) move state machine operations to another task.
    -   [dcd18c53](https://github.com/datafuselabs/openraft/commit/dcd18c5387e723f2619e695407edd81eeaefa547) getting a snapshot does not block `RaftCore` task.
    -   [6eeb5246](https://github.com/datafuselabs/openraft/commit/6eeb5246e145a6a5f62e9ba483dcaa6c346fcbb6) reduce rate to flush metrics.
    -   [1f3bf203](https://github.com/datafuselabs/openraft/commit/1f3bf2037151a33a005469cdcb07a62e46551c34) create a channel `notify` specifically for interal messages.
    -   [54154202](https://github.com/datafuselabs/openraft/commit/54154202beec3e2de433044baae505cc80db375b) move receiving snapshot chunk to sm::Worker.
    -   [fa4085b9](https://github.com/datafuselabs/openraft/commit/fa4085b936900d8b1b49ba1773dc860bc24db6c5) build snapshot in anohter task.
    -   [47048a53](https://github.com/datafuselabs/openraft/commit/47048a535a2cde3580f47efc7c5b8728e0e7549c) `IntoIterator::IntoIter` should be `Send`.
    -   [8dae9ac6](https://github.com/datafuselabs/openraft/commit/8dae9ac6c95a5ae1a7045450aad3d3db57e0c3bf) impl `Clone` for `Raft` does not require `Clone` for its type parameters.
- Fixed:
    -   [cd31970d](https://github.com/datafuselabs/openraft/commit/cd31970ddd83188d2611da9e285621246cff1ef4) trait `RaftLogId` should be public.
    -   [26dc8837](https://github.com/datafuselabs/openraft/commit/26dc8837f4f5afb9be30a32b3eae90235546ca3a) `ProgressEntry::is_log_range_inflight()` checks a log range, not a log entry.
    -   [04e40606](https://github.com/datafuselabs/openraft/commit/04e4060674ba96a4e4244488971f0d9907198051) if the application does not persist snapshot, build a snapshot when starting up.
    -   [3433126c](https://github.com/datafuselabs/openraft/commit/3433126c28c54e307967ad01c4464d5db45ad246) `compat07::SnapshotMeta` should decode v08 `SnapshotMeta`.
    -   [b97edb49](https://github.com/datafuselabs/openraft/commit/b97edb49fadbe8ed1ae725e9a85bd7fa2e044c9a) incorrect debug level log.
    -   [d012705d](https://github.com/datafuselabs/openraft/commit/d012705d8bbeb2e820447fd475a493b6831b5d3c) replication should be able to shutdown when replicating snapshot to unreachable node.
    -   [f505d7e6](https://github.com/datafuselabs/openraft/commit/f505d7e6ddc63c2526d1af2a9acf806b526ce410) `Raft::add_learner()` should block forever.
- Changed:
    -   [a92499f2](https://github.com/datafuselabs/openraft/commit/a92499f20360f0f6eba3452e6944702d6a50f56d) `StoreBuilder` does not need to run a test, it only needs to build a store.
    -   [6e9d3573](https://github.com/datafuselabs/openraft/commit/6e9d35736a0967f06d9e334a0286208e7d6fe123) remove `Clone` from trait `AppData`.
    -   [285e6225](https://github.com/datafuselabs/openraft/commit/285e6225d3e0e8363d8b194e09a113f4e9750b81) instead of a slice, `RaftStorage::append_to_log()` now accepts an `IntoIterator`.
    -   [e0569988](https://github.com/datafuselabs/openraft/commit/e05699889e18e45eeb021ffdf8ca7924d8c218a3) remove unused trait `RaftStorageDebug`.
    -   [88f947a6](https://github.com/datafuselabs/openraft/commit/88f947a663c14dbd41ec39eee1d8d472c38d5706) remove defensive check utilities.
    -   [eaf45dfa](https://github.com/datafuselabs/openraft/commit/eaf45dfa3542340fe52a9796108513637e31e521) move `RaftStateMachine` out of `RaftStorage`.
    -   [9f8ae43e](https://github.com/datafuselabs/openraft/commit/9f8ae43e868c6d8518449dc331d8ad45833ef5bc) `RaftMetrics.replication` type to `BTreeMap<NodeId, Option<LogId>>`.
    -   [84539cb0](https://github.com/datafuselabs/openraft/commit/84539cb03b95ad96875b58961b2d29d9268f2f41) move snapshot type definition from storage traits to `RaftTypeConfig`.
    -   [e78bbffe](https://github.com/datafuselabs/openraft/commit/e78bbffe6abf9a7197d20a609c98de037b731906) remove unused error `CommittedAdvanceTooMany`.

Detail:

### Added:

-   Added: [e39da9f0](https://github.com/datafuselabs/openraft/commit/e39da9f022f5411b8828fac096cd8bcd118d6426) define custom `Entry` type for raft log; by 张炎泼; 2023-03-16

    This commit introduces a new feature that allows applications to define
    a custom type for Raft log entries in `RaftTypeConfig`. By setting `Entry =
    MyEntry`, where `MyEntry` implements the `RaftEntry` trait, an application
    can now define its own log entry type that reflects its architecture.
    However, the default implementation, the `Entry` type is still available.

    This change provides more flexibility for applications to tailor the
    Raft log entry to their specific needs.

    - Fix: #705

    - Changes: `RaftStorage::append_to_log()` and `RaftStorage::apply_to_state_machine()` accepts slice of entries instead of slice of entry references.

-   Added: [87d62d56](https://github.com/datafuselabs/openraft/commit/87d62d5666af0a0d7f7ffa056057f53f1402d33e) add feature flag storage-v2 to enable `RaftLogStorage` and `RaftStateMachine`; by 张炎泼; 2023-04-19

    `storage-v2`: enables `RaftLogStorage` and `RaftStateMachine` as the v2 storage
    This is a temporary feature flag, and will be removed in the future, when v2 storage is stable.
    This feature disables `Adapter`, which is for v1 storage to be used as v2.
    V2 storage separates log store and state machine store so that log IO and state machine IO can be parallelized naturally.

-   Added: [229f3368](https://github.com/datafuselabs/openraft/commit/229f33689fd949eca9fa29fadf2e1285fcb21365) add backoff strategy for unreachable nodes; by 张炎泼; 2023-04-21

    Implements a backoff strategy for temporarily or permanently unreachable nodes.
    If the `Network` implementation returns `Unreachable` error, Openraft
    will backoff for a while before sending next RPC to this target.
    This mechanism prevents error logging flood.

    Adds a new method `backoff()` to `RaftNetwork` to let an application
    return a customized backoff policy, the default provided backoff just
    constantly sleep 500ms.

    Adds an `unreachable_nodes` setting to the testing router `TypedRaftRouteryped` to emulate unreachable nodes.
    Add new error `Unreachable` and an `RPCError` variant `Unreachable`.

    - Fix: #462

-   Added: [f0dc0eb7](https://github.com/datafuselabs/openraft/commit/f0dc0eb7f7fa18d5f546615c51c0f74815457449) add `RaftMetrics::purged` to report the last purged log id; by 张炎泼; 2023-05-01

-   Added: [37e96482](https://github.com/datafuselabs/openraft/commit/37e96482e4f446586af55827cbfff51102f2058e) add `Wait::purged()` to wait for purged to become the expected; by 张炎泼; 2023-05-01

-   Added: [0b419eb6](https://github.com/datafuselabs/openraft/commit/0b419eb619b15139df50798deca61e61998e4c41) add `RaftMetrics.vote`, `Wait::vote()`; by 张炎泼; 2023-05-02

    The latest approved value of `Vote`, which has been saved to disk, is
    referred to as `RaftMetrics.vote`. Additionally, a new `vote()` method
    has been included in `Wait` to enable the application to wait for `vote`
    to reach the anticipated value.

-   Added: [ee7b8853](https://github.com/datafuselabs/openraft/commit/ee7b8853d935ce5f47848d8f62b09e5c83c05c69) new `RaftNetwork` API with argument `RCPOption`; by 张炎泼; 2023-05-02

    - `RaftNetwork` introduced 3 new API `append_entries`,
      `install_snapshot` and `vote` which accept an additional argument
      `RPCOption`, and deprecated the old API `send_append_entries`,
      `send_install_snapshot` and `send_vote`.

    - The old API will be **removed** in `0.9`. An application can still
      implement the old API without any changes. Openraft calls only the new
      API and the default implementation will delegate to the old API.

    - Implementing the new APIs will disable the old APIs.

    - The new APIs accepts an additional argument `RPCOption`, to enable an
      application control the networking behaviors based on the parameters
      in `RPCOption`.

      The `hard_ttl()` and `soft_ttl()` in `RPCOption` sets the hard limit
      and the moderate limit of the duration for which an RPC should run.
      Once the `soft_ttl()` ends, the RPC implementation should start to
      gracefully cancel the RPC, and once the `hard_ttl()` ends, Openraft
      will terminate the ongoing RPC at once.

    - Fix: #819

-   Added: [1ee82cb8](https://github.com/datafuselabs/openraft/commit/1ee82cb82ed9a33015e23fe4ba571a42323cc7cd) `RaftNetwork::send_append_entries()` can return `PartialSuccess`; by 张炎泼; 2023-05-03

    If there are too many log entries and the `RPCOption.ttl` is not
    sufficient, an application can opt to only send a portion of the
    entries, with `AppendEntriesResponse::PartialSuccess(Option<LogId>)`, to
    inform Openraft with the last replicated log id. Thus replication can
    still make progress.

    For example, it tries to send log entries `[1-2..3-10]`, the application
    is allowed to send just `[1-2..1-3]` and return `PartialSuccess(1-3)`,

    ### Caution

    The returned matching log id must be **greater than or equal to** the
    first log id(`AppendEntriesRequest::prev_log_id`) of the entries to
    send. If no RPC reply is received, `RaftNetwork::send_append_entries`
    **must** return an `RPCError` to inform Openraft that the first log
    id(`AppendEntriesRequest::prev_log_id`) may not match on the remote
    target node.

    - Fix: #822

-   Added: [e9eed210](https://github.com/datafuselabs/openraft/commit/e9eed210fc2977c9e6e94a6fc1dd2cadf51362e7) leader lease; by 张炎泼; 2023-05-19

    The leader records the most recent time point when an RPC is initiated
    towards a target node. The highest timestamp associated with RPCs made
    to a quorum serves as the starting time point for a leader lease.

    Improve: use tokio::Instant to replace TimeState

    Use `Instant` for timekeeping instead of a custom `TimeState` struct.
    Because multiple components need to generate timestamp, not only the
    `RaftCore`, e.g., the `ReplicationCore`. And generating a timestamp is
    not in the hot path, therefore caching it introduces unnecessary
    complexity.

-   Added: [269d221c](https://github.com/datafuselabs/openraft/commit/269d221c0fd43cab4a4064de342943e58adf2757) add `SnapshotPolicy::Never`; by 张炎泼; 2023-05-24

    With `SnapshotPolicy::Never`, Openraft will not build snapshots
    automatically based on a policy. Instead, the application has full
    control over when snapshots are built. In this scenario, the application
    can call the `Raft::trigger_snapshot()` API at the desired times to
    manually trigger Openraft to build a snapshot.

    Rename integration tests:
    - `log_compaction -> snapshot_building`
    - `snapshto -> snapshot_streaming`

    -  Fix: #851

-   Added: [9e7195a1](https://github.com/datafuselabs/openraft/commit/9e7195a1f37ff09f45a4f97f2b6a0473c322c604) add `Raft::purge_log()`; by 张炎泼; 2023-05-24

    This method allows users to purge logs when required.
    It initiates the log purge up to and including the given `upto` log index.

    Logs that are not included in a snapshot will **NOT** be purged.
    In such scenario it will delete as many log as possible.
    The `max_in_snapshot_log_to_keep` config is not taken into account when
    purging logs.

    Openraft won't purge logs at once, e.g. it may be delayed by several
    seconds, because if it is a leader and a replication task has been
    replicating the logs to a follower, the logs can't be purged until the
    replication task is finished.

    - Fix: #852

### Improved:

-   Improved: [dbac91d5](https://github.com/datafuselabs/openraft/commit/dbac91d5dc26fd92a8ac5038220176823d9d6d29) send `AppendEntries` response before committing entries; by 张炎泼; 2023-04-04

    When a follower receives an append-entries request that includes a
    series of log entries to append and the log id that the leader
    has committed, it responds with an append-entries response after
    committing and applying the entries.

    However, this is not strictly necessary. The follower could simply send
    the response as soon as the log entries have been appended and flushed
    to disk, without waiting for them to be committed.

-   Improved: [6769cdd0](https://github.com/datafuselabs/openraft/commit/6769cdd09c045757b19cbb3222225b86db28e936) move state machine operations to another task; by 张炎泼; 2023-04-13

    State machine operations, such as applying log entries, building/installing/getting snapshot are moved to `core::sm::Worker`, which is run in a standalone task other than the one running `RaftCore`.
    In this way, log io operation(mostly appending log entries) and state machine io operations(mostly applying log entries) can be paralleled.

    - Log io are sitll running in `RaftCore` task.

    - Snapshot receiving/streaming are removed from `RaftCore`.

    - Add `IOState` to `RaftState` to track the applied log id.

      This field is used to determine whether a certain command, such as
      sending a response, can be executed after a specific log has been
      applied.

    - Refactor: `leader_step_down()` can only be run when the response of the second change-membership is sent.
      Before this commit, updating the `committed` is done atomically with
      sending back response. Since thie commit, these two steps are done
      separately, because applying log entries are moved to another task.
      Therefore `leader_step_down()` must wait for these two steps to be
      finished.

-   Improved: [dcd18c53](https://github.com/datafuselabs/openraft/commit/dcd18c5387e723f2619e695407edd81eeaefa547) getting a snapshot does not block `RaftCore` task; by 张炎泼; 2023-04-16

    `RaftCore` no longer blocks on receiving a snapshot from the state-machine
    worker while replicating a snapshot. Instead, it sends the `Receiver` to
    the replication task and the replication task blocks on receiving the
    snapshot.

-   Improved: [6eeb5246](https://github.com/datafuselabs/openraft/commit/6eeb5246e145a6a5f62e9ba483dcaa6c346fcbb6) reduce rate to flush metrics; by 张炎泼; 2023-04-23

    The performance increases by 40% with this optimization:

    | clients | put/s       | ns/op      | Changes                    |
    | --:     | --:         | --:        | :--                        |
    |  64     | **652,000** |    1,532   | Reduce metrics report rate |
    |  64     | **467,000** |    2,139   |                            |

-   Improved: [1f3bf203](https://github.com/datafuselabs/openraft/commit/1f3bf2037151a33a005469cdcb07a62e46551c34) create a channel `notify` specifically for interal messages; by 张炎泼; 2023-04-25

    `tx_notify` will be used for components such as state-machine worker or
    replication stream to send back notification when an action is done.

    `tx_api` is left for receiving only external messages, such as
    append-entries request or client-write request.

    A `Balancer` is added to prevent one channel from starving the others.

    The benchmark shows a better performance with 64 clients:

    | clients | put/s       | ns/op      | Changes         |
    | --:     | --:         | --:        | :--             |
    |  64     | **730,000** |    1,369   | This commit     |
    |  64     | **652,000** |    1,532   | Previous commit |

-   Improved: [54154202](https://github.com/datafuselabs/openraft/commit/54154202beec3e2de433044baae505cc80db375b) move receiving snapshot chunk to sm::Worker; by 张炎泼; 2023-04-27

    Receiving snapshot chunk should not be run in RaftCore task.
    Otherwise it will block RaftCore.

    In this commit this task is moved to sm::Worker, running in another
    task. The corresponding responding command will not be run until
    sm::Worker notify RaftCore receiving is finished.

-   Improved: [fa4085b9](https://github.com/datafuselabs/openraft/commit/fa4085b936900d8b1b49ba1773dc860bc24db6c5) build snapshot in anohter task; by 张炎泼; 2023-05-02

    Before this commit, snapshot is built in the `sm::Worker`, which blocks
    other state-machine writes, such as applying log entries.

    This commit parallels applying log entries and building snapshot: A
    snapshot is built in another `tokio::task`.

    Because building snapshot is a read operation, it does not have to
    block the entire state machine. Instead, it only needs a consistent view
    of the state machine or holding a lock of the state machine.

    - Fix: #596

-   Improved: [47048a53](https://github.com/datafuselabs/openraft/commit/47048a535a2cde3580f47efc7c5b8728e0e7549c) `IntoIterator::IntoIter` should be `Send`; by 张炎泼; 2023-06-16

    The `RaftStateMachine::apply()` and `RaftLogStorage::append_to_log()`
    method contains a `Send` bound on the `IntoIterator` passed to it.
    However, the actual iterator returned from `IntoIterator` doesn't have
    it. Thus, it's impossible to iterate across awaits in the
    implementation.

    The correct API should be:

    ```
    async fn apply<I>(&mut self, entries: I) -> Result<...>
    where
        I: IntoIterator<Item = C::Entry> + Send,
        I::IntoIter: Send;
    ```

    Thanks to [schreter](https://github.com/schreter)

    - Fix: #860

-   Improved: [8dae9ac6](https://github.com/datafuselabs/openraft/commit/8dae9ac6c95a5ae1a7045450aad3d3db57e0c3bf) impl `Clone` for `Raft` does not require `Clone` for its type parameters; by 张炎泼; 2023-06-16

    Thanks to [xDarksome](https://github.com/xDarksome)

    - Fix: #870

### Fixed:

-   Fixed: [cd31970d](https://github.com/datafuselabs/openraft/commit/cd31970ddd83188d2611da9e285621246cff1ef4) trait `RaftLogId` should be public; by 张炎泼; 2023-03-21

-   Fixed: [26dc8837](https://github.com/datafuselabs/openraft/commit/26dc8837f4f5afb9be30a32b3eae90235546ca3a) `ProgressEntry::is_log_range_inflight()` checks a log range, not a log entry; by 张炎泼; 2023-04-12

    This bug causes replication tries to send pruged log.

-   Fixed: [04e40606](https://github.com/datafuselabs/openraft/commit/04e4060674ba96a4e4244488971f0d9907198051) if the application does not persist snapshot, build a snapshot when starting up; by 张炎泼; 2023-04-15

-   Fixed: [3433126c](https://github.com/datafuselabs/openraft/commit/3433126c28c54e307967ad01c4464d5db45ad246) `compat07::SnapshotMeta` should decode v08 `SnapshotMeta`; by 张炎泼; 2023-04-15

-   Fixed: [b97edb49](https://github.com/datafuselabs/openraft/commit/b97edb49fadbe8ed1ae725e9a85bd7fa2e044c9a) incorrect debug level log; by 张炎泼; 2023-04-22

    This results in unnecessary debug log output.

-   Fixed: [d012705d](https://github.com/datafuselabs/openraft/commit/d012705d8bbeb2e820447fd475a493b6831b5d3c) replication should be able to shutdown when replicating snapshot to unreachable node; by 张炎泼; 2023-05-01

    If a replication is sending a snapshot, it should
    periodically verify the input channel's status. When the input channel
    is closed during replication rebuilding, it should immediately exit the
    loop instead of attempting retries indefinitely.

    - Fix: #808

-   Fixed: [f505d7e6](https://github.com/datafuselabs/openraft/commit/f505d7e6ddc63c2526d1af2a9acf806b526ce410) `Raft::add_learner()` should block forever; by 张炎泼; 2023-05-20

    The `Raft::add_learner()` method, when invoked with the `blocking`
    parameter set to `true`, should block forever until the learner
    synchronizes its logs with the leader.

    In its current implementation, `add_learner()` calls the `Raft::wait()`
    method, which has a default timeout of `500ms`. To achieve the desired
    blocking behavior, the default timeout should be increased
    significantly.

    - Fix: #846

### Changed:

-   Changed: [a92499f2](https://github.com/datafuselabs/openraft/commit/a92499f20360f0f6eba3452e6944702d6a50f56d) `StoreBuilder` does not need to run a test, it only needs to build a store; by 张炎泼; 2023-03-21

    `StoreBuilder::run_test()` is removed, and a new method `build()` is
    added. To test a `RaftStorage` implementation, just implementing
    `StoreBuilder::build()` is enough now. It returns a store instance and a
    **guard**, for disk backed store to clean up the data when the guard is
    dropped.

-   Changed: [6e9d3573](https://github.com/datafuselabs/openraft/commit/6e9d35736a0967f06d9e334a0286208e7d6fe123) remove `Clone` from trait `AppData`; by 张炎泼; 2023-03-26

    Application data `AppData` does not have to be `Clone`.

    Upgrade tip:

    Nothing needs to be done.
    The default `RaftEntry` implementation `Entry` provided by openraft is
    still `Clone`, if the AppData in it is `Clone`.

-   Changed: [285e6225](https://github.com/datafuselabs/openraft/commit/285e6225d3e0e8363d8b194e09a113f4e9750b81) instead of a slice, `RaftStorage::append_to_log()` now accepts an `IntoIterator`; by 张炎泼; 2023-03-27

    Using an `IntoIterator` is more generic than using a slice, and
    could avoid potential memory allocation for the slice.

    Upgrade tip:

    Update the method signature in the `RaftStorage` implementation and
    ensure that it compiles without errors.
    The method body may require minimal modifications as as the new input
    type is just a more general type.

-   Changed: [e0569988](https://github.com/datafuselabs/openraft/commit/e05699889e18e45eeb021ffdf8ca7924d8c218a3) remove unused trait `RaftStorageDebug`; by 张炎泼; 2023-04-10

    `RaftStorageDebug` has only one method `get_state_machine()`,
    and state machine is entirely a user defined struct. Obtaining a state
    machine does not imply anything about the struct or behavior of it.

-   Changed: [88f947a6](https://github.com/datafuselabs/openraft/commit/88f947a663c14dbd41ec39eee1d8d472c38d5706) remove defensive check utilities; by 张炎泼; 2023-04-11

    Most defensive checks are replaced with `debug_assert!` embedded in Engine.
    `StoreExt` as a `RaftStorage` wrapper that implements defensive checks
    are no longer needed. `StoreExt` are mainly used for testing and it is
    very slow so that can not be used in production.

    - Remove structs: `StoreExt`, `DefensiveStoreBuilder`
    - Remove traits: `Wrapper`, `DefensiveCheckBase`, `DefensiveCheck`,

-   Changed: [eaf45dfa](https://github.com/datafuselabs/openraft/commit/eaf45dfa3542340fe52a9796108513637e31e521) move `RaftStateMachine` out of `RaftStorage`; by 张炎泼; 2023-04-01

    In Raft, the state machine is an independent storage component that
    operates separately from the log store. As a result, accessing the log
    store and accessing the state machine can be naturally parallelized.

    This commit replaces the type parameter `RaftStorage` in
    `Raft<.., S: RaftStorage>` with two type parameters: `RaftLogStorage` and
    `RaftStateMachine`.

    - Add: `RaftLogReaderExt` to provide additional log access methods based
      on a `RaftLogReader` implementation. Some of the methods are moved
      from `StorageHelper` to this trait.

    - Add: `Adapter` to let application use the seperated log state machine
      framework without rewriting `RaftStorage` implementation.

    - Refactor: shorten type names for the 2 example crates

    ### Upgrade tip

    Use an adapter to wrap `RaftStorage`:
    ```rust
    // Before:
    let store = MyRaftStorage::new();
    Raft::new(..., store);

    // After:
    let store = MyRaftStorage::new();
    let (log_store, sm) = Adaptoer::new(store);
    Raft::new(..., log_store, sm);
    ```

-   Changed: [9f8ae43e](https://github.com/datafuselabs/openraft/commit/9f8ae43e868c6d8518449dc331d8ad45833ef5bc) `RaftMetrics.replication` type to `BTreeMap<NodeId, Option<LogId>>`; by 张炎泼; 2023-04-24

    The `RaftMetrics.replication` used to be of type
    `ReplicationMetrics{ replication: BTreeMap<NodeId, ReplicationTargetMetrics> }`
    which contained an atomic log index value for each
    ReplicationTargetMetrics stored in the `BTreeMap`. The purpose of this
    type was to reduce the cost of copying a metrics instance. However,
    since the metrics report rate has been significantly reduced, this
    cost is now negligible. As a result, these complicated implementations
    have been removed. When reporting metrics, they can simply be cloned
    from the progress information maintained by `Engine`.

    ### Upgrade tip

    Replace usage of `RaftMetrics.replication.data().replication.get(node_id)` with
    `RaftMetrics.replication.get(node_id)`.

-   Changed: [84539cb0](https://github.com/datafuselabs/openraft/commit/84539cb03b95ad96875b58961b2d29d9268f2f41) move snapshot type definition from storage traits to `RaftTypeConfig`; by 张炎泼; 2023-04-26

    Similar to `NodeId` or `Entry`, `SnapshotData` is also a data type that
    is specified by the application and needs to be defined in
    `RaftTypeConfig`, which is a collection of all application types.

    Public types changes:

    - Add `SnapshotData` to `RaftTypeConfig`:
      ```rust
      pub trait RaftTypeConfig {
          /// ...
          type SnapshotData: AsyncRead + AsyncWrite + AsyncSeek + Send + Sync + Unpin + 'static;
      }
      ```
    - Remove associated type `SnapshotData` from `storage::RaftStorage`.
    - Remove associated type `SnapshotData` from `storage::v2::RaftStateMachine`.

    Corresponding API changes:

    - Change `storage::RaftSnapshotBuilder<C: RaftTypeConfig, SNAPSHOT_DATA>` to `RaftSnapshotBuilder<C>`
    - Change `storage::Snapshot<NID: NodeId, N: Node, SNAPSHOT_DATA>` to `storage::Snapshot<C>`

    Upgrade tip:

    Update generic type parameter in application types to pass compilation.

-   Changed: [e78bbffe](https://github.com/datafuselabs/openraft/commit/e78bbffe6abf9a7197d20a609c98de037b731906) remove unused error `CommittedAdvanceTooMany`; by 张炎泼; 2023-05-14

    Upgrade tip:

    Do not use it.

## v0.8.3

### Improved:

-   Improved: [23f4a73b](https://github.com/datafuselabs/openraft/commit/23f4a73b2382d55deac849db1767519a91c23796) AppDataResponse does not need a Clone trait bound; by 张炎泼; 2023-03-09

    - Fix: #703

-   Improved: [664635e0](https://github.com/datafuselabs/openraft/commit/664635e0e6d29ea4fa84e6f579d0c785f3a513e7) loosen validity check with RaftState.snapshot_last_log_id(); by 张炎泼; 2023-03-10

    A application may not persist snapshot. And when it restarted, the
    last-purged-log-id is not `None` but `snapshot_last_log_id()` is None.
    This is a valid state and should not emit error.

-   Improved: [54aea8a2](https://github.com/datafuselabs/openraft/commit/54aea8a267741ca458c86ef1885041d244817c86) fix: delay election if a greater last log id is seen; by 张炎泼; 2023-03-14

    If this node sees a greater last-log-id on another node, it will be less
    likely to be elected as a leader.
    In this case, it is necessary to sleep for a longer period of time
    `smaller_log_timeout` so that other nodes with a greater last-log-id
    have a chance to elect themselves.

    Fix: such as state should be kept until next election, i.e., it should
    be a field of `Engine` instead of a `field` of `internal_server_state`.
    And this value should be greater than the `election_timeout` of every other node.

### Changed:

-   Changed: [9ddb5715](https://github.com/datafuselabs/openraft/commit/9ddb5715a33902a83124f48ee33d75d490fcffa5) RaftState: make `RaftState.vote` private. Accesses vote via 2 new public methods: `vote_ref()` and `vote_last_modified()`.; by 张炎泼; 2023-03-12

-   Changed: [3b4f4e18](https://github.com/datafuselabs/openraft/commit/3b4f4e186bca5404744d2940571974500d52734d) move log id related traits to mod `openraft::log_id`; by 张炎泼; 2023-03-14

    Move trait `RaftLogId`, `LogIdOptionExt` and `LogIndexOptionExt` from `openraft::raft_types` to mod `openraft::log_id`

## v0.8.2

### Changed:

-   Changed: [342d0de2](https://github.com/datafuselabs/openraft/commit/342d0de2b885388dfa5de64430384bd3275d3697) rename variants in ChangeMembers, add `AddVoters`; by 张炎泼; 2023-03-01

    Rename `ChangeMembers::AddVoter` to `AddVoterIds`, because it just
    updates voter ids.

    Rename `ChangeMembers::RemoveVoter` to `RemoveVoters`.

    Add `ChangeMembers::AddVoters(BTreeMap)` to add voters with
    corresponding `Node`, i.e., it adds nodes as learners and update
    the voter-ids in a `Membership`.

### Added:

-   Added: [50821c37](https://github.com/datafuselabs/openraft/commit/50821c37035850dba9e237d9e7474e918f2bd410) impl PartialEq for Entry; by 张炎泼; 2023-03-02

### Fixed:

-   Fixed: [97fa1581](https://github.com/datafuselabs/openraft/commit/97fa15815a7d51c35a3a613b11defbf5f49cf4c1) discard blank log heartbeat, revert to the standard heartbeat; by 张炎泼; 2023-03-04

    - https://github.com/datafuselabs/openraft/issues/698

    The blank log heartbeat design has two problems:

    - The heartbeat that sends a blank log introduces additional I/O, as a follower has to persist every log to maintain correctness.

    - Although `(term, log_index)` serves as a pseudo time in Raft, measuring whether a node has caught up with the leader and is capable of becoming a new leader, leadership is not solely determined by this pseudo time.
      Wall clock time is also taken into account.

      There may be a case where the pseudo time is not upto date but the clock time is, and the node should not become the leader.
      For example, in a cluster of three nodes, if the leader (node-1) is busy sending a snapshot to node-2(it has not yet replicated the latest logs to a quorum, but node-2 received message from the leader(node-1), thus it knew there is an active leader), node-3 should not seize leadership from node-1.
      This is why there needs to be two types of time, pseudo time `(term, log_index)` and wall clock time, to protect leadership.

      In the follow graph:
      - node-1 is the leader, has 4 log entries, and is sending a snapshot to
          node-2,
      - node-2 received several chunks of snapshot, and it perceived an active
          leader thus extended leader lease.
      - node-3 tried to send vote request to node-2, although node-2 do not have
          as many logs as node-3, it should still reject node-3's vote request
          because the leader lease has not yet expired.

      In the obsolete design, extending pseudo time `(term, index)` with a
      `tick`, in this case node-3 will seize the leadership from node-2.

      ```text
      Ni: Node i
      Ei: log entry i

      N1 E1 E2 E3 E4
            |
            v
      N2    snapshot
            +-----------------+
                     ^        |
                     |        leader lease
                     |
      N3 E1 E2 E3    | vote-request
      ---------------+----------------------------> clock time
                     now

      ```

    The original document is presented below for reference.

-   Fixed: [b5caa44d](https://github.com/datafuselabs/openraft/commit/b5caa44d1aac0b539180c1c490f0883dcc83048a) Wait::members() should not count learners as members; by 张炎泼; 2023-03-04

    `Wait::members()` waits until membership becomes the expected value.
    It should not check against all nodes.
    Instead, it should only check voters, excluding learners.

## v0.8.1

### Added:

-   Added: [b3c2ff7e](https://github.com/datafuselabs/openraft/commit/b3c2ff7e37ce5996f572f43bc574cb50b2d0cdc2) add Membership methods: voter_ids(), learner_ids(), get_node(); by 张炎泼; 2023-02-28

## v0.8.0

### Fixed:

-   Fixed: [86e2ccd0](https://github.com/datafuselabs/openraft/commit/86e2ccd071454f32b04dd4446c0eece6d4075580) a single Candidate should be able to vote itself.; by 张炎泼; 2022-01-20

    A Candidate should check if it is the only member in a cluster before
    sending vote request.
    Otherwise a single node cluster does work.

-   Fixed: [4015cc38](https://github.com/datafuselabs/openraft/commit/4015cc388c9259af7399bfc89715deb0c1c9f9bb) a Candidate should revert to Follower at once when a higher vote is seen; by 张炎泼; 2022-02-03

    When a Candidate saw a higher vote, it store it at once.
    Then no more further granted votes are valid to this candidate,
    because vote they granted are changed.

    Thus it was wrong to compare `last_log_id` before deciding if to revert to
    Follower.
    The right way is to revert to Follower at once and stop the voting
    procedure.

-   Fixed: [1219a880](https://github.com/datafuselabs/openraft/commit/1219a8801b5a7a2b7f2af10719852cb696e42903) consistency issue between ReplicationCore.last_log_id and last_log_state.last_log_id; by 张炎泼; 2022-02-28

-   Fixed: [efdc321d](https://github.com/datafuselabs/openraft/commit/efdc321dad07344360732da277f63150e5cfc4d0) a leader should report leader metrics with value `Update::AsIs` instead of `Update::Update(None)`. Otherwise it mistakenly purges metrics about replication; by 张炎泼; 2022-04-01

-   Fixed: [797fb9b1](https://github.com/datafuselabs/openraft/commit/797fb9b1a8d2ed4216f07a791a3cf72242442711) update replication metrics only when the replication task stopped, to provide a consistent view of RaftMetrics; by 张炎泼; 2022-06-04

-   Fixed: [918b48bc](https://github.com/datafuselabs/openraft/commit/918b48bcbf27aae83a8c4e63fb10ad3b33520ea1) #424 wrong range when searching for membership entries: `[end-step, end)`.; by 张炎泼; 2022-07-03

    The iterating range searching for membership log entries should be
    `[end-step, end)`, not `[start, end)`.
    With this bug it will return duplicated membership entries.

    - Related: #424

-   Fixed: [8594807c](https://github.com/datafuselabs/openraft/commit/8594807c4cd15f5ac3459471fd33e548e94dd660) metrics has to be updated last; by 张炎泼; 2022-07-13

    Otherwise the application receives updated metrics while the internal raft
    state is still stale.

-   Fixed: [59ddc982](https://github.com/datafuselabs/openraft/commit/59ddc982100a390efd66632adbf25edd2c6e6a3c) avoid creating log-id with uninitialized `matched.leader_id`.; by 张炎泼; 2022-07-26

    When waiting for a newly added learner to become up to date,
    it tries to compare last-log-id and the reported `matched` replication
    state.
    But the `matched` may have not yet receive any update and is
    uninitialized, in such case, it tries to create a temp LogId with
    `leader_id(0, 0)`, which is illegal.

    The fix is simple: do not use log-id. Just calculating replication lag by log index.

    Add test to reproduce it: openraft/tests/membership/t99_issue_471_adding_learner_uses_uninit_leader_id.rs

    - Fix: #471

-   Fixed: [43dd8b6f](https://github.com/datafuselabs/openraft/commit/43dd8b6f1afd2febcc027505c5ee41775ad561a8) when leader reverts to follower, send error to waiting clients; by 张炎泼; 2022-08-06

    When a leader reverts to follower, e.g., if a higher vote is seen,
    it should inform waiting clients that leadership is lost.

-   Fixed: [71a290cd](https://github.com/datafuselabs/openraft/commit/71a290cd0a41c80e0bbbb455baa976a7f2945bc9) when handling append-entries, if `prev_log_id` is purged, it should not treat it as a **conflict**; by 张炎泼; 2022-08-14

    when handling append-entries, if `prev_log_id` is purged, it
    should not treat it as a **conflict** log and should not delete any
    log.

    This bug is caused by using `committed` as `last_applied`.
    `committed` may be smaller than `last_applied` when a follower just
    starts up.

    The solution is merging `committed` and `last_applied` into one field:
    `committed`, which is always greater than or equal the actually
    committed(applied).

-   Fixed: [674e78aa](https://github.com/datafuselabs/openraft/commit/674e78aa171626db0369c1f025e641abdc8f7264) potential inconsistency when installing snapshot; by 张炎泼; 2022-09-21

    The conflicting logs that are before `snapshot_meta.last_log_Id` should
    be deleted before installing a snapshot.

    Otherwise there is chance the snapshot is installed but conflicting logs
    are left in the store, when a node crashes.

-   Fixed: [4ea66acd](https://github.com/datafuselabs/openraft/commit/4ea66acd35f998251bced1ff25b40db8781d8d4b) stop tick task when shutting down Raft; by Matthias Wahl; 2022-09-27

-   Fixed: [56486a60](https://github.com/datafuselabs/openraft/commit/56486a60c63fa5da1b8fdb877306c725058b1892) Error after change_membership: `assertion failed: value > prev`: #584; by 张炎泼; 2022-10-29

    Problem:

    Error occurs after calling `change_membership()`: `assertion failed: value > prev`,
    when changing membership by converting a learner to a voter.

    Because the replication streams are re-spawned, thus progress reverts to
    zero. Then a reverted progress causes the panic.

    Solution:

    When re-spawning replications, remember the previous progress.

    - Fix: #584

-   Fixed: [678af4a8](https://github.com/datafuselabs/openraft/commit/678af4a8191400a2936979bc809a7c7d37fbe660) when responding ForwardToLeader, make `leader_id` a None if the leader is no longer in the cluster; by 张炎泼; 2022-11-02

-   Fixed: [0023cff1](https://github.com/datafuselabs/openraft/commit/0023cff188df7654bf2e4e8980cc83307e93ec71) delay leader step down; by 张炎泼; 2022-11-06

    When a membership that removes the leader is committed,
    the leader continue to work for a short while before reverting to a learner.
    This way, let the leader replicate the `membership-log-is-committed` message to followers.

    Otherwise, if the leader step down at once, the follower might have to re-commit the membership log
    again.

    After committing the membership log that does not contain the leader,
    the leader will step down in the next `tick`.

-   Fixed: [ff9a9335](https://github.com/datafuselabs/openraft/commit/ff9a93357506b3f53d74046e87d887d8551e4d3b) it should make a node non-leader when restarting single node cluster; by 张炎泼; 2022-12-03

    A node should not set `server_state` to `Leader` when just starting up,
    even when it's the only voter in a cluster. It still needs several step
    to initialize leader related fields to become a leader.

    - Fix: #607

-   Fixed: [0e7ab5a7](https://github.com/datafuselabs/openraft/commit/0e7ab5a70877d72407942a2639c2f24bca64a48a) workaround cargo leaking SSL_CERT_FILE issue; by 张炎泼; 2022-12-09

    On Linux: command `cargo run` pollutes environment variables: It leaks
    `SSL_CERT_FILE` and `SSL_CERT_DIR` to the testing sub progress it runs.
    Which cause `reqwest` spending ~50 ms loading the certificates for every
    RPC.

    We just extend the RPC timeout to work around.

    - Fix: #550

-   Fixed: [cc8af8cd](https://github.com/datafuselabs/openraft/commit/cc8af8cd67d78118e0ea48dc5d1de3adf183e45a) last_purged_log_id is not loaded correctly; by 张炎泼; 2023-01-08

    - Fix: `last_purged_log_id` should be `None`, but not `LogId{index=0,
      ..}` when raft startup with a store with log at index 0.

      This is fixed by adding another field `next_purge` to distinguish
      `last_purged_log_id` value `None` and `LogId{index=0, ..}`, because
      `RaftState.log_ids` stores `LogId` but not `Option<LogId>`.

    - Add a wrapper `Valid<RaftState>` of `RaftState` to check if the state
      is valid every time accessing it. This check is done only when
      `debug_assertions` is turned on.

-   Fixed: [9dbbe14b](https://github.com/datafuselabs/openraft/commit/9dbbe14b91eee9e2ce4497a6c5c10f5df2e5913b) check_is_leader() should return at once if encountering StorageError; by 张炎泼; 2023-02-12

    Refactor: ExtractFatal is not used any more. Fatal error should only be
    raised by Command executor, no more by API handler. There is no need to
    extract Fatal error from an API error.

-   Fixed: [a80579ef](https://github.com/datafuselabs/openraft/commit/a80579efec75e8655b55e2532f81f38757419bcd) a stepped down leader should ignore replication progress message; by 张炎泼; 2023-02-12

-   Fixed: [c8fccb22](https://github.com/datafuselabs/openraft/commit/c8fccb2225862370ac5e4e7e27c9632f82f332d1) when adding a learner, ensure the last membership is committed; by 张炎泼; 2023-02-19

    Previously, when adding a learner to a Raft cluster, the last membership was not
    always marked as committed, which could cause issues when a follower tried to
    truncate logs by reverting to the last committed membership. To prevent this
    issue, we have updated the code to ensure the last membership is committed when
    adding a learner.

    In addition to this fix, we have also made several refactoring changes,
    including refining method names for trait `Coherent`, renaming
    `Membership::next_safe()` to `next_coherent()` for consistency, and updating
    enum `ChangeMembers` to include more variants for adding and removing learners.
    We have also removed `RaftCore::add_learner()` in favor of using
    `change_membership()` for all membership operations, and added a `ChangeHandler`
    to build new membership configurations for change-membership requests.

    Finally, we have updated the `Membership` API with a new method `new_with_nodes()`
    for building a new membership configuration, and moved the validation check
    out into a separate function, `ensure_valid()`. Validation is now done only
    when needed.

### Changed:

-   Changed: [86e2ccd0](https://github.com/datafuselabs/openraft/commit/86e2ccd071454f32b04dd4446c0eece6d4075580) `Wait::log_at_least()` use `Option<u64>` as the input log index, instead of using u64; by 张炎泼; 2022-01-20

-   Changed: [71a290cd](https://github.com/datafuselabs/openraft/commit/71a290cd0a41c80e0bbbb455baa976a7f2945bc9) remove `RaftState.last_applied`, use `committed` to represent the already committed and applied log id; by 张炎泼; 2022-08-14

-   Changed: [2254ffc5](https://github.com/datafuselabs/openraft/commit/2254ffc563946149eca1e0907993b2efdba8f65d) add sub error types of ReplicationError; by 张炎泼; 2022-01-20

    - Add sub errors such as Timeout and NetworkError.

    - Remove ReplicationError::IO, use StorageError instead.

-   Changed: [f08a3e6d](https://github.com/datafuselabs/openraft/commit/f08a3e6d09c85f3a10f033d94bf0213a723ad008) RaftNetwork return `RPCError` instead of anyhow::Error; by 张炎泼; 2022-01-23

    - When a remote error encountered when replication, the replication will
      be stopped at once.

    - Fix: #140

-   Changed: [d55fa625](https://github.com/datafuselabs/openraft/commit/d55fa62575cbf369b018e151b654b6a8905fdd87) add ConfigError sub error; remove anyhow; by 张炎泼; 2022-01-23

    - Fix: #144

-   Changed: [58f2491f](https://github.com/datafuselabs/openraft/commit/58f2491f3a6a90c95730fee4c176f34e526049db) `RaftStorage`: use `Vote` to replace `HardState`; by 张炎泼; 2022-01-25

    - Rename: save_hard_state() and read_hard_state() to save_vote() and read_vote().

    - Replace `term, node_id` pair with `Vote` in RaftCore and RPC struct-s.

-   Changed: [a68a9a9a](https://github.com/datafuselabs/openraft/commit/a68a9a9af9c8d32e81e51b974ec62c22bdce8048) use `term, node_id, index` to identify a log entry; by 张炎泼; 2022-01-26

-   Changed: [0b753622](https://github.com/datafuselabs/openraft/commit/0b7536221a7214756abf844049eae037b3c9ccfc) `Raft::add_learner()` accepts optional arg `Node`.; by 张炎泼; 2022-02-17

    When adding a learner, an optional `Node` can be provided to store
    additional info of a node in Membership.

    A common usage if to store node address in the Membership so that an
    application does not need another component to get address of a node
    when implementing `RaftNetwork`.

-   Changed: [5ba730c9](https://github.com/datafuselabs/openraft/commit/5ba730c96acffad122c33bca916a4d4034b3ef1d) Replace replication state in RaftMetrics with a reference to atomic values; by Ivan Schréter; 2022-02-22

-   Changed: [a76f41ac](https://github.com/datafuselabs/openraft/commit/a76f41ac058939d95f2df53013be3b68dc5b68ad) Extract RaftLogReader, RaftSnapshotBuilder from RaftStorage, split RaftNetwork and RaftNetworkFactory; by Ivan Schréter; 2022-02-22

    RaftStorage is now refactored to:
    - RaftLogReader to read data from the log in parallel tasks independent of the main Raft loop
    - RaftStorage to modify the log and the state machine (implements also RaftLogReader) intended to be used in the main Raft loop
    - RaftSnapshotBuilder to build the snapshot in background independent of the main Raft loop

    The RaftStorage API offers to create new RaftLogReader or RaftSnapshotBuilder on it.

    RaftNetwork is also refactored to:
    - RaftNetwork responsible for sending RPCs
    - RaftNetworkFactory responsible for creating instances of RaftNetwork for sending data to a particular node

-   Changed: [f40c2055](https://github.com/datafuselabs/openraft/commit/f40c205532b4e75384a6eaaebb601447001d13f4) Add a `RaftTypeConfig` trait to configure common types; by Ivan Schréter; 2022-02-25

-   Changed: [650e2352](https://github.com/datafuselabs/openraft/commit/650e23524b759219b9dea0d2fbe3e71859429115) Membership remove redundant field `learners`: the node ids that are in `Membership.nodes` but not in `Membership.configs` are learners; by 张炎泼; 2022-03-07

-   Changed: [81cd3443](https://github.com/datafuselabs/openraft/commit/81cd3443530a9ef260ded00f0a8b2825cefe8196) EffectiveMembership.log_id to Option<LogId>; by 张炎泼; 2022-04-05

-   Changed: [67375a2a](https://github.com/datafuselabs/openraft/commit/67375a2a44cb06a84815df1119c28dcf2681f842) RaftStorage: use `EffectiveMembership` instead of `Option<_>`; by 张炎泼; 2022-04-05

-   Changed: [ffc82682](https://github.com/datafuselabs/openraft/commit/ffc8268233d87eaccd6099e77340f706c0b5c3cc) rename ReplicationMetrics and methods in MetricsChangeFlags; by 张炎泼; 2022-04-05

    - Change: rename ReplicationMetrics to ReplicationTargetMetrics

    - Change: rename LeaderMetrics to ReplicationMetrics

-   Changed: [7b1d4660](https://github.com/datafuselabs/openraft/commit/7b1d4660d449d6f35230d0253e9564612cdfb7e0) rename RaftMetrics.leader_metrics to replication; by 张炎泼; 2022-04-06

-   Changed: [30b485b7](https://github.com/datafuselabs/openraft/commit/30b485b744647fd4d5ca711d202a1dc0c59e2aff) rename State to ServerState; by 张炎泼; 2022-04-16

-   Changed: [ca8a09c1](https://github.com/datafuselabs/openraft/commit/ca8a09c1898dbcaa4c2bf49bf5dabc5221e0b908) rename InitialState to RaftState; by 张炎泼; 2022-04-16

-   Changed: [8496a48a](https://github.com/datafuselabs/openraft/commit/8496a48a87373eab13117d1e62d4d2faf42918ca) add error `Fatal::Panicked`, storing RaftCore panic; by 张炎泼; 2022-05-09

    Changes:

    - Add `committed_membership` to RaftState, to store the previous
      committed membership config.

    - Change: `RaftStorage::get_membership()` returns a vec of at most 2 memberships.

    - Change: `RaftStorage::last_membership_in_log()` returns a vec of at most 2 memberships.

-   Changed: [1f645feb](https://github.com/datafuselabs/openraft/commit/1f645feb3ff4a747d944213c431ebb37f36e4d6b) add `last_membership` to `SnapshotMeta`; by 张炎泼; 2022-05-12

-   Changed: [bf4e0497](https://github.com/datafuselabs/openraft/commit/bf4e049762c5cf333381363942260f27a435432d) Make serde optional; by devillve084; 2022-05-22

-   Changed: [b96803cc](https://github.com/datafuselabs/openraft/commit/b96803ccd085a19a21fc7a99451543d6ef1dee1d) `external_request()` replace the 1st arg ServerState with RaftState; by 张炎泼; 2022-06-08

    This change let user do more things with a external fn request.

-   Changed: [d81c7279](https://github.com/datafuselabs/openraft/commit/d81c72792b3a3441df6953b3f174e82a8f3b6985) after shutdown(), it should return an error when accessing Raft, instead of panicking.; by devillve084; 2022-06-16

-   Changed: [0de003ce](https://github.com/datafuselabs/openraft/commit/0de003ce732122437be0e116f10fcf94a8731075) remove `RaftState.last_log_id` and `RaftState.last_purged_log_id`; by 张炎泼; 2022-06-22

    Remove these two fields, which are already included in
    `RaftState.log_ids`; use `last_log_id()` and `last_purged_log_id()`
    instead.

-   Changed: [7f00948d](https://github.com/datafuselabs/openraft/commit/7f00948d5e1d09d9c2e61dd5c69cd2f6ef3cddbe) API: cleanup APIs in Membership and EffectiveMembership; by 张炎泼; 2022-06-29

    - Refactor: move impl of `QuorumSet` from `Membership` to `EffectiveMembership`.

      Add a field `EffectiveMembership.quorum_set`, to store a
      `QuorumSet` built from the `Membership` config. This quorum set can have
      a different structure from the `Membership`, to optimized quorum check.

    - Refactor: impl methods in `Membership` or `EffectiveMembership` with
      Iterator if possible.

    - Refactor: use term `voter` and `learner` for methods and fields.

-   Changed: [01a16d08](https://github.com/datafuselabs/openraft/commit/01a16d0814f52ca05dbd2d7876d759bb3d696b33) remove `tx` from `spawn_replication_stream()`; by 张炎泼; 2022-07-01

    Replication should not be responsible invoke the callback when
    replication become upto date. It makes the logic dirty.
    Such a job can be done by watching the metrics change.

    - Change: API: AddLearnerResponse has a new field `membership_log_id`
      which is the log id of the membership log that contains the newly
      added learner.

-   Changed: [6b9ae52f](https://github.com/datafuselabs/openraft/commit/6b9ae52fa98ea09c20d30a67759c40e09ab2e407) remove error `AddLearnerError::Exists`; by 张炎泼; 2022-07-01

    Even when the learner to add already exists, the caller may still want
    to block until the replication catches up. Thus it does not expect an
    error.

    And `Exists` is not an issue the caller has to deal with, it does not
    have to be an error.

-   Changed: [d7afc721](https://github.com/datafuselabs/openraft/commit/d7afc721414ac3e55c84f8ae304ad6ea8db1b697) move default impl methods in `RaftStorage` to `StorageHelper`.; by 张炎泼; 2022-07-01

    - `get_initial_state()`
    - `get_log_id()`
    - `get_membership()`
    - `last_membership_in_log()`

    In the trait `RaftStorage`, these methods provide several default
    methods that users do not need to care about. It should no longer
    be methods that user may need to implement.

    To upgrade:

    If you have been using these methods, replace `sto.xxx()` with
    `StorageHelper::new(&mut sto).xxx()`.

-   Changed: [a010fddd](https://github.com/datafuselabs/openraft/commit/a010fddda6294ee3155e15df15a6b67bd27b33a1) Stop replication to removed node at once when new membership is seen; by 张炎泼; 2022-07-12

    Before this commit, when membership changes, e.g., from a joint config
    `[(1,2,3), (3,4,5)]` to uniform config `[3,4,5]`(assuming the leader is
    `3`), the leader stops replication to `1,2` when `[3,4,5]` is
    committed.

    This is an unnecessarily complicated solution.
    It is OK for the leader to stop replication to `1,2` as soon as config `[3,4,5]` is seen, instead of when config `[3,4,5]` is committed.

    - If the leader(`3`) finally committed `[3,4,5]`, it will eventually stop replication to `1,2`.
    - If the leader(`3`) crashes before committing `[3,4,5]`:
      - And a new leader sees the membership config log `[3,4,5]`, it will continue to commit it and finally stop replication to `1,2`.
      - Or a new leader does not see membership config log `[3,4,5]`, it will re-establish replication to `1,2`.

    In any case, stopping replication at once is OK.

    One of the considerations about this modification is:
    The nodes, e.g., `1,2` do not know they have been removed from the cluster:

    - Removed node will enter the candidate state and keeps increasing its term and electing itself.
      This won't affect the working cluster:

      - The nodes in the working cluster have greater logs; thus, the election will never succeed.

      - The leader won't try to communicate with the removed nodes thus it won't see their higher `term`.

    - Removed nodes should be shut down finally. No matter whether the
      leader replicates the membership without these removed nodes to them,
      there should always be an external process that shuts them down.
      Because there is no guarantee that a removed node can receive the
      membership log in a finite time.

    Changes:

    - Change: remove config `remove_replication`, since replication will
      be removed at once.

    - Refactor: Engine outputs `Command::UpdateReplicationStream` to inform
      the Runtime to update replication, when membership changes.

    - Refactor: remove `ReplicationState.failures`, replication does not
      need count failures to remove it.

    - Refactor: remove `ReplicationState.matched`: the **matched** log id
      has been tracked by `Engine.state.leader.progress`.

    - Fix: #446

-   Changed: [2d1aff03](https://github.com/datafuselabs/openraft/commit/2d1aff03bbf264de0d43f0e84db10a968febf1c6) error InProgress: add field `committed`; by 张炎泼; 2022-07-15

    - Refactor: Simplify Engine command executor

-   Changed: [8c7f0857](https://github.com/datafuselabs/openraft/commit/8c7f08576db05d3a5e62236c77b32436afb2e6f8) remove ClientWriteRequest; by 张炎泼; 2022-08-01

    Remove struct `ClientWriteRequest`.
    `ClientWriteRequest` is barely a wrapper that does not provide any
    additional function.

    `Raft::client_write(ClientWriteRequest)` is changed to
    `Raft::client_write(app_data: D)`, where `D` is application defined
    `AppData` implementation.

-   Changed: [565b6921](https://github.com/datafuselabs/openraft/commit/565b692102ac8810b7b67a87c04a619110da8fc1) `ErrorSubject::Snapshot(SnapshotSignature)`; by 张炎泼; 2022-08-02

    Change `ErrorSubject::Snapshot(SnapshotMeta)` to `ErrorSubject::Snapshot(SnapshotSignature)`.

    `SnapshotSignature` is the same as `SnapshotMeta` except it does not include
    `Membership` information.
    This way errors do not have to depend on type `Node`, which is used in
    `Membership` and it is a application specific type.

    Then when a user-defined generic type `NodeData` is introduced, error
    types do not need to change.

    - Part of: #480

-   Changed: [e4b705ca](https://github.com/datafuselabs/openraft/commit/e4b705cabbc417b2ef6569a31b2ba83a71cac41e) Turn `Node` into a trait (#480); by Heinz N. Gies; 2022-08-03

    Structs that depend on `Node` now have to implement `trait Node`,  or use a predefined basic implementation `BasicNode`. E.g., `struct Membership` now has two type parameters: `impl<NID, N> Membership<NID, N> where N: Node, NID: NodeId`.

-   Changed: [c836355a](https://github.com/datafuselabs/openraft/commit/c836355a10cc90c7ede0062e29c0c21214667271) `Membership.nodes` remove `Option` from value; by 张炎泼; 2022-08-04

    Before this commit, the value of `Membership.nodes` is `Option<N:
    Node>`: `Membership.nodes: BTreeMap<NID, Option<N>>`

    The value does not have to be an `Option`.
    If an application does not need openraft to store the `Node` data, it
    can just implement `trait Node` with an empty struct, or just use
    `BasicNode` as a placeholder.

    - Using `Option<N>` as the value is a legacy and since #480 is merged, we
      do not need the `Option` any more.

-   Changed: [70e3318a](https://github.com/datafuselabs/openraft/commit/70e3318abb13aca60b91f98dfcedba2bcb99ee0e) SnapshotMeta.last_log_id from LogId to Option of LogId; by 张炎泼; 2022-08-17

    `SnapshotMeta.last_log_id` should be the same type as
    `StateMachine.last_applied`.

    By making `SnapshotMeta.last_log_id` an Option of LogId, a snapshot can
    be build on an empty state-machine(in which `last_applied` is None).

-   Changed: [d0d04b28](https://github.com/datafuselabs/openraft/commit/d0d04b28fedc10b05a41d818ff7e3de77f246cb8) only purge logs that are in snapshot; by 张炎泼; 2022-08-28

    Let `snapshot+logs` be a complete state of a raft node.

    The Assumption before is `state_machine+logs` is a complete state of a
    raft node. This requires state machine to persist the state every time
    applying a log, which would be an innecessary overhead.

    - Change: remove ENV config entries. Do not let a lib be affected by
      environment variables.

    - Change: remove `Config.keep_unsnapshoted_log`: now by default, logs
      not included in snapshot won't be deleted.

      Rename `Config.max_applied_log_to_keep` to `max_in_snapshot_log_to_keep`.

-   Changed: [3111e7e6](https://github.com/datafuselabs/openraft/commit/3111e7e6c8649f7068f959bc58e484f3b95732bb) RaftStorage::install_snapshot() does not need to return state changes; by 张炎泼; 2022-08-28

    The caller of `RaftStorage::install_snapshot()` knows about what changes
    have been made, the return value is unnecessary.

-   Changed: [a12fd8e4](https://github.com/datafuselabs/openraft/commit/a12fd8e410ca8cf33a9c264224640b1b1045e41e) remove error MissingNodeInfo; by 张炎泼; 2022-11-02

    Because in a membership the type `Node` is not an `Option` any more,
    `MissingNodeInfo` error will never occur.

-   Changed: [dbeae332](https://github.com/datafuselabs/openraft/commit/dbeae332190b7724297d50834b6583de11009923) rename `IntoOptionNodes` to `IntoNodes`; by 张炎泼; 2022-11-02

-   Changed: [e8ec9c50](https://github.com/datafuselabs/openraft/commit/e8ec9c50b2f17e8e021481e61406c74c7c26adaa) EffectiveMembership::get_node() should return an Option; by 张炎泼; 2022-11-02

    `EffectiveMembership::get_node()` should return an `Option<&Node>`
    instead of a `&Node`.
    Otherwise it panic if the node is not found.

-   Changed: [93116312](https://github.com/datafuselabs/openraft/commit/9311631264074307974a61d9b37e6c5026983abc) remove error NodeNotFound; by 张炎泼; 2022-12-28

    A node is stored in `Membership` thus it should always be found.
    Otherwise it is a bug of openraft.
    In either case, there is no need for an application to deal with
    `RPCError::NodeNotFound` error.

    An application that needs such an error should define it as an
    application error.

    - Migration guide:
      if you do have been using it, you could just replace `NodeNotFound` with `NetworkError`.

    - Fix: #623

-   Changed: [e1238428](https://github.com/datafuselabs/openraft/commit/e123842862ac91860352afc48a48f19f011e7ab7) RaftState: add field snapshot_meta; by 张炎泼; 2022-12-30

    Snapshot meta should be part of the `RaftState`.
    Move it from `Engine` to `RaftState`

-   Changed: [2dd81018](https://github.com/datafuselabs/openraft/commit/2dd81018b54a74d46af6412048b2f057bb35c56e) make Raft::new() async and let it return error during startup; by 张炎泼; 2023-01-02

    - Change: move startup process from `RaftCore::do_main()` to `Raft::new()`, so
      that an error during startup can be returned earlier.

      Upgrade guide: application has to consume the returned future with
      `Raft::new().await`, and the error returned by the future.

    - Refactor: move id from `Engine.id` to `Engine.config.id`, so that accessing
      constant attribute does not depend on a reference to `Engine`.

-   Changed: [3d5e0016](https://github.com/datafuselabs/openraft/commit/3d5e00169962bac198c71b46db61cccbae59fa9b) A restarted leader should enter leader state at once, without another round of election; by 张炎泼; 2023-01-04

    - Test: single-node restart test does not expect the node to run
      election any more.

    - Refactor: add VoteHandler to handle vote related operations.

    - Change: make ServerState default value `Learner`.

    - Fix: #607

-   Changed: [77e87a39](https://github.com/datafuselabs/openraft/commit/77e87a39401af0cd2f8b1f74b3aeb5962e8e6715) remove InitializeError::NotAMembershipEntry error; by 张炎泼; 2023-02-12

    Such an error can only be caused by internal calls. An application do
    not need to handle it.

-   Changed: [fbb3f211](https://github.com/datafuselabs/openraft/commit/fbb3f211a043e5d5468426334943e189c5a6d8ff) add RaftError as API return error type.; by 张炎泼; 2023-02-12

    Add `RaftError<E>` as error type returned by every `Raft::xxx()` API.
    RaftError has two variants: Fatal error or API specific error.
    This way every API error such as AppendEntriesError does not have to include
    an `Fatal` in it.

    Upgrade tip:

    The affected types is mainly `trait RaftNetwork`, an application should
    replace AppendEntriesError, VoteError, InstallSnapshotError with
    `RaftError<_>`, `RaftError<_>`, and `RaftError<_, InstallSnapshotError>`.

    So is for other parts, e.g., `Raft::append_entries()` now returns
    `Result<AppendEntriesResponse, RaftError<_>>`, an application should
    also rewrite error handling that calls these APIs.

    See changes in examples/.

-   Changed: [d1b3b232](https://github.com/datafuselabs/openraft/commit/d1b3b23219433ff594d7249e086bd1e95ef5b8e5) remove RaftNetworkFactory::ConnectionError and AddLearnerError::NetworkError; by 张炎泼; 2023-02-12

    `RaftNetworkFactory::new_client()` does not return an error because
    openraft can only ignore it.  Therefore it should **not** create a
    connection but rather a client that will connect when required.  Thus
    there is chance it will build a client that is unable to send out
    anything, e.g., in case the Node network address is configured
    incorrectly.

    Because of the above change, And `AddLearnerError` will not include a
    NetworkError any more, because when adding a learner, the connectivity
    can not be effectively detected.

    Upgrade tip:

    Just update the application network implementation so that it compiles.

-   Changed: [0161a3d2](https://github.com/datafuselabs/openraft/commit/0161a3d21f70f19fb12994cda950ccaea93dc8b4) remove AddLearnerResponse and AddLearnerError; by 张炎泼; 2023-02-17

    In openraft adds a learner is done by committing a membership config
    log, which is almost the same as committing any log.

    `AddLearnerResponse` contains a field `matched` to indicate the
    replication state to the learner, which is not included in
    `ClientWriteResponse`. This information can be retrieved via
    `Raft::metrics()`.

    Therefore to keep the API simple, replace `AddLearnerResponse` with
    `ClientWriteResponse`.

    Behavior change: adding a learner always commit a new membership config
    log, no matter if it already exists in membership.
    To avoid duplicated add, an application should check existence first by
    examining `Raft::metrics()`

    - Fix: #679

    Upgrade tips:

    - Replace AddLearnerResponse with ClientWriteResponse
    - Replace AddLearnerError with ClientWriteError

    Passes the application compilation.

    See the changes in examples/.

-   Changed: [9906d6e9](https://github.com/datafuselabs/openraft/commit/9906d6e9c01f1aa73e747f863dfe0a866915045f) remove non-blocking membership change; by 张炎泼; 2023-02-18

    When changing membership in nonblocking mode, the leader submits a
    membership config log but does not wait for the log to be committed.

    This is useless because the caller has to assert the log is
    committed, by periodically querying the metrics of a raft node, until it
    is finally committed. Which actually makes it a blocking routine.

    API changes:

    - Removes `allow_lagging` parameter from `Raft::change_membership()`
    - Removes error `LearnerIsLagging`

    Upgrade tip:

    Adjust API calls to make it compile.

    Refactor: move `leader_append_entries()` to `LeaderHandler`.

-   Changed: [f591726a](https://github.com/datafuselabs/openraft/commit/f591726a1a9e51b88c31c1228fa1034b62d1e777) trait IntoNodes adds two new method has_nodes() and node_ids(); by 张炎泼; 2023-02-19

    `trait IntoNodes` converts types `T` such as `Vec` or `BTreeSet` into
    `BTreeMap<NID, Node>`.

    This patch changes the functionality of the `IntoNodes` trait to provide
    two new methods `has_nodes()` and `node_ids()`, in addition to the existing
    `into_nodes()` method. The `has_nodes()` method returns true if the type `T`
    contains any `Node` objects, and `node_ids()` returns a `Vec` of the `NodeId`
    objects associated with the `Node` objects in `T`.

    Refactor:

    The patch also refactors the `Membership::next_safe()` method to return an
    `Err(LearnerNotFound)` if it attempts to build a `Membership` object
    containing a `voter_id` that does not correspond to any `Node`.

-   Changed: [55217aa4](https://github.com/datafuselabs/openraft/commit/55217aa4786bee66b5cfac2662e1770434afb73f) move default implemented method from trait `RaftLogReader` to `StorageHelper`; by 张炎泼; 2023-02-21

    Function `get_log_entries()` and `try_get_log_entry()` are provided by
    trait `RaftLogReader` with default implementations. However, they do not
    need to be part of this trait and an application does not have to
    implement them.

    Therefore in this patch they are moved to `StorageHelper` struct, which
    provides additional storage access methods that are built based on the
    `RaftStorage` trait.

-   Changed: [0a1dd3d6](https://github.com/datafuselabs/openraft/commit/0a1dd3d69557f412f7e8ecd5b903408308090a96) replace EffectiveMembership with StoredMembership in RaftStorage; by 张炎泼; 2023-02-26

    `EffectiveMembership` is a struct used at runtime, which contains
    additional information such as an optimized `QuorumSet` implementation
    that has different structure from a `Membership`.

    To better separate concerns, a new struct called `StoredMembership` has
    been introduced specifically for storage purpose. It contains only the
    information that needs to be stored in storage. Therefore,
    `StoredMembership` is used instead of `EffectiveMembership` in
    RaftStorage.

    Upgrade tip:

    Replace `EffectiveMembership` with `StoredMembership` in an application.

    Fields in `EffectiveMembership` are made private and can be accessed via
    corresponding methods such as: `EffectiveMembership.log_id` and
    `EffectiveMembership.membership` should be replaced with
    `EffectiveMembership::log_id()` and `EffectiveMembership::membership()`.

### Added:

-   Added: [966eb287](https://github.com/datafuselabs/openraft/commit/966eb287ff9bc98112b7b5d69253cca2d86277f8) use a version to track metrics change; by 张炎泼; 2022-03-03

    Add `Versioned<D>` to track changes of an `Arc<D>`.

    In openraft, some frequently updated object such metrics are wrapped
    in an `Arc`, and some modification is made in place: by storing an
    `AtomicU64`.

-   Added: [80f89134](https://github.com/datafuselabs/openraft/commit/80f89134533be18675c5686e1b9787777096f624) Add support for external requests to be executed inside of Raft core loop; by Ivan Schréter; 2022-03-05

    The new feature is also exposed via `RaftRouter` test fixture and tested in the initialization test (in addition to the original checks).

-   Added: [2a5c1b9e](https://github.com/datafuselabs/openraft/commit/2a5c1b9e8de7e96e79b917429e0ea70a23ef51ae) add feature-flag: `bt` enables backtrace; by 张炎泼; 2022-03-12

-   Added: [16406aec](https://github.com/datafuselabs/openraft/commit/16406aec6c4bcbe3818e2409d56aa74f534dead9) add error NotAllowed; by 张炎泼; 2022-04-06

-   Added: [a8655446](https://github.com/datafuselabs/openraft/commit/a86554469c8d7efc28d68c6fd799aa588c08f53f) InitialState: add `last_purged_log_id`; by 张炎泼; 2022-04-07

-   Added: [6f20e1fc](https://github.com/datafuselabs/openraft/commit/6f20e1fc80fa59cc758eff731663f3becc89671c) add trait `RaftPayload` `RaftEntry` to access payload and entry without the need to know about user data, i.e., `AppData` or `AppDataResponse`.; by 张炎泼; 2022-04-07

-   Added: [67c870e2](https://github.com/datafuselabs/openraft/commit/67c870e2f5141e55d9107942f452f26f2c030cdd) add err: NotAMembershipEntry, NotInMembers; by 张炎泼; 2022-04-16

-   Added: [675a0f8f](https://github.com/datafuselabs/openraft/commit/675a0f8f09720fd1d74635024f3d72aa1cf3b2cf) Engine stores log ids; by 张炎泼; 2022-04-16

-   Added: [2262c79f](https://github.com/datafuselabs/openraft/commit/2262c79f5195307402e7a0994771b4152c0d10b2) LogIdList: add method purge() to delete log ids; by 张炎泼; 2022-05-08

-   Added: [ff898cde](https://github.com/datafuselabs/openraft/commit/ff898cdef508f411a57780a919788beab5ff3fea) Engine: add method: purge_log(); by 张炎泼; 2022-05-08

-   Added: [4d0918f2](https://github.com/datafuselabs/openraft/commit/4d0918f261d32e28593ac81da87e7cf4e7ccb752) Add rocks based example; by Heinz N. Gies; 2022-07-05

-   Added: [86eb2981](https://github.com/datafuselabs/openraft/commit/86eb29812b9433cc6f25ce21fa873447bf94fe2e) Raft::enable_tick() to enable or disable election timeout; by 张炎泼; 2022-07-31

-   Added: [956177df](https://github.com/datafuselabs/openraft/commit/956177df8f57e46240ae1abf283bdd49aa9d8b85) use blank log for heartbeat (#483); by 张炎泼; 2022-08-01

    * Feature: use blank log for heartbeat

    Heartbeat in standard raft is the way for a leader to assert it is still alive.
    - A leader send heartbeat at a regular interval.
    - A follower that receives a heartbeat believes there is an active leader thus it rejects election request(`send_vote`) from another node unreachable to the leader, for a short period.

    Openraft heartbeat is a blank log

    Such a heartbeat mechanism depends on clock time.
    But raft as a distributed consensus already has its own **pseudo time** defined very well.
    The **pseudo time** in openraft is a tuple `(vote, last_log_id)`, compared in dictionary order.

    Why it works

    To refuse the election by a node that does not receive recent messages from the current leader,
    just let the active leader send a **blank log** to increase the **pseudo time** on a quorum.

    Because the leader must have the greatest **pseudo time**,
    thus by comparing the **pseudo time**, a follower automatically refuse election request from a node unreachable to the leader.

    And comparing the **pseudo time** is already done by `handle_vote_request()`,
    there is no need to add another timer for the active leader.

    Other changes:

    - Feature: add API to switch timeout based events:
      - `Raft::enable_tick()`: switch on/off election and heartbeat.
      - `Raft::enable_heartbeat()`: switch on/off heartbeat.
      - `Raft::enable_elect()`: switch on/off election.

      These methods make some testing codes easier to write.
      The corresponding `Config` entries are also added:
      `Config::enable_tick`
      `Config::enable_heartbeat`
      `Config::enable_elect`

    - Refactor: remove Engine `Command::RejectElection`.
      Rejecting election now is part of `handle_vote_req()` as blank-log
      heartbeat is introduced.

    - Refactor: heartbeat is removed from `ReplicationCore`.
      Instead, heartbeat is emitted by `RaftCore`.

    - Fix: when failed to sending append-entries, do not clear
      `need_to_replicate` flag.

    - CI: add test with higher network delay.

    - Doc: explain why using blank log as heartbeat.

    - Fix: #151

-   Added: [b6817758](https://github.com/datafuselabs/openraft/commit/b6817758c19c3aaf43b9dafd7612b2a1ca897a51) add `Raft::trigger_elect()` and `Raft::trigger_heartbeat()` to let user manually trigger a election or send a heartbeat log; by 张炎泼; 2022-08-06

-   Added: [f437cda0](https://github.com/datafuselabs/openraft/commit/f437cda09c0112f63913cbea29a7b02c66d5e897) add Raft::trigger_snapshot() to manually trigger to build snapshot at once; by 张炎泼; 2022-08-07

-   Added: [eae08515](https://github.com/datafuselabs/openraft/commit/eae085155ac58b87749fdd1d57febb26e4d14835) Added sled store example based on rocks example; by kus; 2022-08-16

-   Added: [07a2a677](https://github.com/datafuselabs/openraft/commit/07a2a6774d909a4d086fd86e7cd2f75ff1fa58eb) adding a snapshot finalize timeout config; by Zach Schoenberger; 2022-11-09

-   Added: [2877be0c](https://github.com/datafuselabs/openraft/commit/2877be0c890d8ef2be9d2ebbd36d0b48d3bd3620) add config: send_snapshot_timeout; by Zach Schoenberger; 2022-11-09

-   Added: [541e9d36](https://github.com/datafuselabs/openraft/commit/541e9d36ba5f26115e2698068ebd23f9bf0236b0) add "Inflight" to store info about inflight replication data; by 张炎泼; 2023-01-17

-   Added: [4a85ee93](https://github.com/datafuselabs/openraft/commit/4a85ee936856ce6bfde4c8cc6c58e15b00971257) feature flag "single-term-leader": standard raft mode; by 张炎泼; 2023-02-13

    With this feature on: only one leader can be elected in each term, but
    reduce LogId size from `LogId:{term, node_id, index}` to `LogId{term, index}`.

    Add `CommittedLeaderId` as the leader-id type used in `LogId`:
    The leader-id used in `LogId` can be different(smaller) from leader-id used
    in `Vote`, depending on `LeaderId` definition.
    `CommittedLeaderId` is the smallest data that can identify a leader
    after the leadership is granted by a quorum(committed).

    Change: Vote stores a LeaderId in it.

    ```rust
    // Before
    pub struct Vote<NID> {
        pub term: u64,
        pub node_id: NID,
        pub committed: bool,
    }

    // After
    pub struct Vote<NID> {
        #[cfg_attr(feature = "serde", serde(flatten))]
        pub leader_id: LeaderId<NID>,

        pub committed: bool,
    }
    ```

    Upgrade tip:

    If you manually serialize `Vote`, i.e. without using
    `serde`, the serialization part should be rewritten.

    Otherwise, nothing needs to be done.

    - Fix: #660

-   Added: [4f4b05f6](https://github.com/datafuselabs/openraft/commit/4f4b05f6e03042030ff44f58c651c86ecdfc8f4a) add v07-v08 compatible store rocksstore-compat07; by 张炎泼; 2023-02-25

## v0.7.4

### Changed:

-   Changed: [1bd22edc](https://github.com/datafuselabs/openraft/commit/1bd22edc0a8dc7a9c314341370b3dfeb357411b5) remove AddLearnerError::Exists, which is not actually used; by 张炎泼; 2022-09-30

-   Changed: [c6fe29d4](https://github.com/datafuselabs/openraft/commit/c6fe29d4a53b47f6c43d83a24e1610788a4c0166) change-membership does not return error when replication lags; by 张炎泼; 2022-10-22

    If `blocking` is `true`, `Raft::change_membership(..., blocking)` will
    block until repliication to new nodes become upto date.
    But it won't return an error when proposing change-membership log.

    - Change: remove two errors: `LearnerIsLagging` and `LearnerNotFound`.

    - Fix: #581

### Fixed:

-   Fixed: [2896b98e](https://github.com/datafuselabs/openraft/commit/2896b98e34825a8623ec4650da405c79827ecbee) changing membership should not remove replication to all learners; by 张炎泼; 2022-09-30

    When changing membership, replications to the learners(non-voters) that
    are not added as voter should be kept.

    E.g.: with a cluster of voters `{0}` and learners `{1, 2, 3}`, changing
    membership to `{0, 1, 2}` should not remove replication to node `3`.

    Only replications to removed members should be removed.

### Added:

-   Added: [9a22bb03](https://github.com/datafuselabs/openraft/commit/9a22bb035b1d456ab2949c8b3abdbaa630622c63) add rocks-store as a `RaftStorage` implementation based on rocks-db; by 张炎泼; 2023-02-22

## v0.7.3

### Changed:

-   Changed: [25e94c36](https://github.com/datafuselabs/openraft/commit/25e94c36e5c8ae640044196070f9a067d5f105a3) InstallSnapshotResponse: replies the last applied log id; Do not install a smaller snapshot; by 张炎泼; 2022-09-22

    A snapshot may not be installed by a follower if it already has a higher
    `last_applied` log id locally.
    In such a case, it just ignores the snapshot and respond with its local
    `last_applied` log id.

    This way the applied state(i.e., `last_applied`) will never revert back.

### Fixed:

-   Fixed: [21684bbd](https://github.com/datafuselabs/openraft/commit/21684bbdfdc54b18daa68f623afc2b0be6718c72) potential inconsistency when installing snapshot; by 张炎泼; 2022-09-22

    The conflicting logs that are before `snapshot_meta.last_log_id` should
    be deleted before installing a snapshot.

    Otherwise there is chance the snapshot is installed but conflicting logs
    are left in the store, when a node crashes.

## v0.7.2

### Added:

-   Added: [568ca470](https://github.com/datafuselabs/openraft/commit/568ca470524f77bc721edb104206e1774e1555cc) add Raft::remove_learner(); by 张炎泼; 2022-09-02

## v0.7.1

### Added:

-   Added: [ea696474](https://github.com/datafuselabs/openraft/commit/ea696474191b82069fae465bb064a2e599537ede) add feature-flag: `bt` enables backtrace; by 张炎泼; 2022-03-12

    `--features bt` enables backtrace when generating errors.
    By default errors does not contain backtrace info.

    Thus openraft can be built on stable rust by default.

    To use on stable rust with backtrace, set `RUSTC_BOOTSTRAP=1`, e.g.:
    ```
    RUSTUP_TOOLCHAIN=stable RUSTC_BOOTSTRAP=1 make test
    ```

## v0.7.0



## v0.7.0-alpha.5



## v0.7.0-alpha.4



## v0.7.0-alpha.3

### Changed:

-   Changed: [f99ade30](https://github.com/datafuselabs/openraft/commit/f99ade30a7f806f18ed19ace12e226cd62fd43ec) API: move default impl methods in RaftStorage to StorageHelper; by 张炎泼; 2022-07-04

### Fixed:

-   Fixed: [44381b0c](https://github.com/datafuselabs/openraft/commit/44381b0c776cfbb7dfc7789de27346110776b7f6) when handling append-entries, if prev_log_id is purged, it should not delete any logs.; by 张炎泼; 2022-08-14

    When handling append-entries, if the local log at `prev_log_id.index` is
    purged, a follower should not believe it is a **conflict** and should
    not delete all logs. It will get committed log lost.

    To fix this issue, use `last_applied` instead of `committed`:
    `last_applied` is always the committed log id, while `committed` is not
    persisted and may be smaller than the actually applied, when a follower
    is restarted.

## v0.7.0-alpha.2

### Fixed:

-   Fixed: [30058c03](https://github.com/datafuselabs/openraft/commit/30058c036de06e9d0d66dd290dc75cf06831e12e) #424 wrong range when searching for membership entries: `[end-step, end)`.; by 张炎泼; 2022-07-03

    The iterating range searching for membership log entries should be
    `[end-step, end)`, not `[start, end)`.
    With this bug it will return duplicated membership entries.

    - Bug: #424

## v0.7.0-alpha.1

### Fixed:

-   Fixed: [d836d85c](https://github.com/datafuselabs/openraft/commit/d836d85c963f6763eb7e5a5c72fb81da3435bdb2) if there may be more logs to replicate, continue to call send_append_entries in next loop, no need to wait heartbeat tick; by lichuang; 2022-01-04

-   Fixed: [5a026674](https://github.com/datafuselabs/openraft/commit/5a026674617b5e4f5402ff148d261169fe392b24) defensive_no_dirty_log hangs tests; by YangKian; 2022-01-08

-   Fixed: [8651625e](https://github.com/datafuselabs/openraft/commit/8651625ed0c18354ff329a37f042f9471f333fa6) save leader_id if a higher term is seen when handling append-entries RPC; by 张炎泼; 2022-01-10

    Problem:

    A follower saves hard state `(term=msg.term, voted_for=None)`
    when a `msg.term > local.term` when handling append-entries RPC.

    This is quite enough to be correct but not perfect. Correct because:

    - In one term, only an established leader will send append-entries;

    - Thus, there is a quorum voted for this leader;

    - Thus, no matter what `voted_for` is saved, it is still correct. E.g.
      when handling append-entries, a follower node could save hard state
      `(term=msg.term, voted_for=Some(ANY_VALUE))`.

    The problem is that a follower already knows the legal leader for a term
    but still does not save it. This leads to an unstable cluster state: The
    test sometimes fails.

    Solution:

    A follower always save hard state with the id of a known legal leader.

-   Fixed: [1a781e1b](https://github.com/datafuselabs/openraft/commit/1a781e1be204ce165248ea0d075880cd06d8fb00) when lack entry, the snapshot to build has to include at least all purged logs; by 张炎泼; 2022-01-18

-   Fixed: [a0a94af7](https://github.com/datafuselabs/openraft/commit/a0a94af7612bd9aaae5a5404325887b16d7a96ae) span.enter() in async loop causes memory leak; by 张炎泼; 2022-06-17

    It is explained in:
    https://onesignal.com/blog/solving-memory-leaks-in-rust/

### Changed:

-   Changed: [c9c8d898](https://github.com/datafuselabs/openraft/commit/c9c8d8987805c29476bcfd3eab5bff83e00e342e) trait RaftStore: remove get_membership_config(), add last_membership_in_log() and get_membership() with default impl; by drdr xp; 2022-01-04

    Goal: minimize the work for users to implement a correct raft application.

    Now RaftStorage provides default implementations for `get_membership()`
    and `last_membership_in_log()`.

    These two methods just can be implemented with other basic user impl
    methods.

    - fix: #59

-   Changed: [abda0d10](https://github.com/datafuselabs/openraft/commit/abda0d1015542ad701f52fb1a6ca7f997a81102a) rename RaftStorage methods do_log_compaction: build_snapshot, delete_logs_from: delete_log; by 张炎泼; 2022-01-15

-   Changed: [a52a9300](https://github.com/datafuselabs/openraft/commit/a52a9300e5be96a73a7c76ce4095f723d8750837) RaftStorage::get_log_state() returns last purge log id; by 张炎泼; 2022-01-16

    -   Change: `get_log_state()` returns the `last_purged_log_id` instead of the `first_log_id`.
        Because there are some cases in which log are empty:
        When a snapshot is install that covers all logs,
        or when `max_applied_log_to_keep` is 0.

        Returning `None` is not clear about if there are no logs at all or
        all logs are deleted.

        In such cases, raft still needs to maintain log continuity
        when repilcating. Thus the last log id that once existed is important.
        Previously this is done by checking the `last_applied_log_id`, which is
        dirty and buggy.

        Now an implementation of `RaftStorage` has to maintain the
        `last_purged_log_id` in its store.

    -   Change: Remove `first_id_in_log()`, `last_log_id()`, `first_known_log_id()`,
        because concepts are changed.

    -   Change: Split `delete_logs()` into two method for clarity:

        `delete_conflict_logs_since()` for deleting conflict logs when the
        replication receiving end find a conflict log.

        `purge_logs_upto()` for cleaning applied logs

    -   Change: Rename `finalize_snapshot_installation()` to `install_snapshot()`.

    -   Refactor: Remove `initial_replicate_to_state_machine()`, which does nothing
        more than a normal applying-logs.

    -   Refactor: Remove `enum UpdateCurrentLeader`. It is just a wrapper of Option.

-   Changed: [7424c968](https://github.com/datafuselabs/openraft/commit/7424c9687f2ae378ff4647326fcd53f2c172b50b) remove unused error MembershipError::Incompatible; by 张炎泼; 2022-01-17

-   Changed: [beeae721](https://github.com/datafuselabs/openraft/commit/beeae721d31d89e3ad98fe66376d8edf57f3dcd0) add ChangeMembershipError sub error for reuse; by 张炎泼; 2022-01-17

## v0.6.4



## v0.6.3



## v0.6.2

### Fixed:

-   Fixed: [4d58a51e](https://github.com/datafuselabs/openraft/commit/4d58a51e41189acba06c1d2a0e8466759d9eb785) a non-voter not in joint config should not block replication; by drdr xp; 2021-08-31

-   Fixed: [eed681d5](https://github.com/datafuselabs/openraft/commit/eed681d57950fc58b6ca71a45814b8f6d2bb1223) race condition of concurrent snapshot-install and apply.; by drdr xp; 2021-09-01

    Problem:

    Concurrent snapshot-install and apply mess up `last_applied`.

    `finalize_snapshot_installation` runs in the `RaftCore` thread.
    `apply_to_state_machine` runs in a separate tokio task(thread).

    Thus there is chance the `last_applied` being reset to a previous value:

    - `apply_to_state_machine` is called and finished in a thread.

    - `finalize_snapshot_installation` is called in `RaftCore` thread and
      finished with `last_applied` updated.

    - `RaftCore` thread finished waiting for `apply_to_state_machine`, and
      updated `last_applied` to a previous value.

    ```
    RaftCore: -.    install-snapshot,         .-> replicate_to_sm_handle.next(),
               |    update last_applied=5     |   update last_applied=2
               |                              |
               v                              |
    task:      apply 2------------------------'
    --------------------------------------------------------------------> time
    ```

    Solution:

    Rule: All changes to state machine must be serialized.

    A temporary simple solution for now is to call all methods that modify state
    machine in `RaftCore` thread.
    But this way it blocks `RaftCore` thread.

    A better way is to move all tasks that modifies state machine to a
    standalone thread, and send update request back to `RaftCore` to update
    its fields such as `last_applied`

-   Fixed: [a48a3282](https://github.com/datafuselabs/openraft/commit/a48a3282c58bdcae3309545a06431aecf3e65db8) handle-vote should compare last_log_id in dictionary order, not in vector order; by drdr xp; 2021-09-09

    A log `{term:2, index:1}` is definitely greater than log `{term:1, index:2}` in raft spec.
    Comparing log id in the way of `term1 >= term2 && index1 >= index2` blocks election:
    no one can become a leader.

-   Fixed: [228077a6](https://github.com/datafuselabs/openraft/commit/228077a66d5099fd404ae9c68f0977e5f978f102) a restarted follower should not wait too long to elect. Otherwise the entire cluster hangs; by drdr xp; 2021-11-19

-   Fixed: [6c0ccaf3](https://github.com/datafuselabs/openraft/commit/6c0ccaf3ab3f262437d8fc021d4b13437fa4c9ac) consider joint config when starting up and committing.; by drdr xp; 2021-12-24

    - Change: MembershipConfig support more than 2 configs

    - Makes fields in MembershipConfig privates.
      Provides methods to manipulate membership.

    - Fix: commit without replication only when membership contains only one
      node. Previously it just checks the first config, which results in
      data loss if the cluster is in a joint config.

    - Fix: when starting up, count all nodes but not only the nodes in the
      first config to decide if it is a single node cluster.

-   Fixed: [b390356f](https://github.com/datafuselabs/openraft/commit/b390356f17327fa65caab2f7120e8c311b95bf75) first_known_log_id() should returns the min one in log or in state machine; by drdr xp; 2021-12-28

-   Fixed: [cd5a570d](https://github.com/datafuselabs/openraft/commit/cd5a570d68bd2d475ef3d1cd9d2203b5bdb15403) clippy warning; by lichuang; 2022-01-02

### Changed:

-   Changed: [deda6d76](https://github.com/datafuselabs/openraft/commit/deda6d7677d2cb866184b316ae29077fe0bc3865) remove PurgedMarker. keep logs clean; by drdr xp; 2021-09-09

    Changing log(add a PurgedMarker(original SnapshotPointer)) makes it
    diffeicult to impl `install-snapshot` for a RaftStore without a lock
    protecting both logs and state machine.

    Adding a PurgedMarker and installing the snapshot has to be atomic in
    storage layer. But usually logs and state machine are separated store.
    e.g., logs are stored in fast flash disk and state machine is stored
    some where else.

    To get rid of the big lock, PurgedMarker is removed and installing a
    snaphost does not need to keep consistent with logs any more.

-   Changed: [734eec69](https://github.com/datafuselabs/openraft/commit/734eec693745a5fc116ec6d6ff3ac8771dd94a9f) VoteRequest: use last_log_id:LogId to replace last_log_term and last_log_index; by drdr xp; 2021-09-09

-   Changed: [74b16524](https://github.com/datafuselabs/openraft/commit/74b16524e828ae9bdf9b9feebdf9f5c50faa9478) introduce StorageError. RaftStorage gets rid of anyhow::Error; by drdr xp; 2021-09-13

    `StorageError` is an `enum` of DefensiveError and StorageIOError.
    An error a RaftStorage impl returns could be a defensive check error
    or an actual io operation error.

    Why:

    anyhow::Error is not enough to support the flow control in RaftCore.
    It is typeless thus RaftCore can not decide what next to do
    depending on the returned error.

    Inside raft, anyhow::Error should never be used, although it could be used as
    `source()` of some other error types.

-   Changed: [46bb3b1c](https://github.com/datafuselabs/openraft/commit/46bb3b1c323da7b5b6a3b476d7ab26ffde835706) `RaftStorage::finalize_snapshot_installation` is no more responsible to delete logs included in snapshot; by drdr xp; 2021-09-13

    A RaftStorage should be as simple and intuitive as possible.

    One should be able to correctly impl a RaftStorage without reading the
    guide but just by guessing what a trait method should do.

    RaftCore is able to do the job of deleting logs that are included in
    the state machine, RaftStorage should just do what is asked.

-   Changed: [2cd23a37](https://github.com/datafuselabs/openraft/commit/2cd23a37013a371cc89c8a969f4fb10f450f2043) use structopt to impl config default values; by drdr xp; 2021-09-14

-   Changed: [ac4bf4bd](https://github.com/datafuselabs/openraft/commit/ac4bf4bddf2c998c6347f3af3d0bc16ecfb3573a) InitialState: rename last_applied_log to last_applied; by drdr xp; 2021-09-14

-   Changed: [74283fda](https://github.com/datafuselabs/openraft/commit/74283fda9c18510ed4cdb0435929180a7dc97585) RaftStorage::do_log_compaction() do not need to delete logs any more raft-core will delete them.; by drdr xp; 2021-09-14

-   Changed: [112252b5](https://github.com/datafuselabs/openraft/commit/112252b506eed376cbdad61b091585f868e1be13) RaftStorage add 2 API: last_id_in_log() and last_applied_state(),  remove get_last_log_id(); by drdr xp; 2021-09-15

-   Changed: [7f347934](https://github.com/datafuselabs/openraft/commit/7f347934d25b5517002ff2843b0517e0b4785cbf) simplify membership change; by drdr xp; 2021-09-16

    - Change: if leadership is lost, the cluster is left with the **joint**
      config.
      One does not receive response of the change-membership request should
      always re-send to ensure membership config is applied.

    - Change: remove joint-uniform logic from RaftCore, which brings a lot
      complexity to raft impl. This logic is now done in Raft(which is a
      shell to control RaftCore).

    - Change: RaftCore.membership is changed to `ActiveMembership`, which
      includes a log id and a membership config.
      Making this change to let raft be able to check if a membership is
      committed by comparing the log index and its committed index.

    - Change: when adding a existent non-voter, it returns an `Ok` value
      instead of an `Err`.

    - Change: add arg `blocking` to `add_non_voter` and `change_membership`.
      A blocking `change_membership` still wait for the two config change
      log to commit.
      `blocking` only indicates if to wait for replication to non-voter to
      be up to date.

    - Change: remove `non_voters`. Merge it into `nodes`.
      Now both voters and non-voters share the same replication handle.

    - Change: remove field `ReplicationState.is_ready_to_join`, it
      can be just calculated when needed.

    - Change: remove `is_stepping_down`, `membership.contains()` is quite
      enough.

    - Change: remove `consensus_state`.

-   Changed: [df684131](https://github.com/datafuselabs/openraft/commit/df684131d1f6e60e611c37310f3ccbd693c5cadb) bsearch to find matching log between leader and follower; by drdr xp; 2021-12-17

    - Refactor: simplify algo to find matching log between leader and follower.
      It adopts a binary-search like algo:

      The leader tracks the max matched log id(`self.matched`) and the least unmatched log id(`self.max_possible_matched_index`).

      The follower just responds if the `prev_log_id` in

      AppendEntriesRequest matches the log at `prev_log_id.index` in its
      store.

      Remove the case-by-case algo.

    - Change: RaftStorage adds 2 new API: `try_get_log_entries()`,
      `first_id_in_log()` and `first_known_log_id()`.

      These a are not stable, may be removed soon.

    - Fix: the timeout for `Wait()` should be a total timeout. Otherwise a
      `Wait()` never quits.

    - Fix: when send append-entries request, if a log is not found, it
      should retry loading, but not enter snapshot state.
      Because a log may be deleted by RaftCore just after Replication read
      `prev_log_id` from the store.

    - Refactor: The two replication loop: line-rate loop and snapshot loop
      should not change the `ReplicationState`, but instead returning an
      error.
      Otherwise it has to check the state everywhere.

    - Refactor: simplify receiving RaftCore messages: split
      `drain_raft_rx()` into `process_raft_event()` and
      `try_drain_raft_rx()`.

    - Feature: a store impl has to add an initial log at index 0 to make the
      store mathematics complete.

    - Feature: add `ReplicationError` to describe all errors that is
      emitted when replicating entries or snapshot.

-   Changed: [6625484c](https://github.com/datafuselabs/openraft/commit/6625484cdaab183cfd015c319c804d96f7b620bc) remove EntryNormal; by drdr xp; 2021-12-23

-   Changed: [61551178](https://github.com/datafuselabs/openraft/commit/61551178b2a8d5f84fa62a88ef5b5966e0f53aab) remove EntryMembership; by drdr xp; 2021-12-23

-   Changed: [c61b4c49](https://github.com/datafuselabs/openraft/commit/c61b4c4922d2cee93135ccd87c95dcd7ab780ad1) remove ConflictOpt, which is a wrapper of log_id; add matched log id in AppendEntriesResponse; by drdr xp; 2021-12-23

-   Changed: [3511e439](https://github.com/datafuselabs/openraft/commit/3511e439e23e90b45ad7daa6274070b07d5fa576) rename MembershipConfig to Membership; by drdr xp; 2021-12-27

-   Changed: [b43c085a](https://github.com/datafuselabs/openraft/commit/b43c085a88710330028ceef7845db1cfd3c3c5b0) track committed log id instead of just a commit index; by drdr xp; 2021-12-29

-   Changed: [8506102f](https://github.com/datafuselabs/openraft/commit/8506102f731fa7ccbc03051a0da0215356245553) remove unused field SnapshotMeta::membership; by drdr xp; 2021-12-29

### Dependency:

-   Dependency: [7848c219](https://github.com/datafuselabs/openraft/commit/7848c219807207956ddf757d6908958bd5a1fe4d) update pretty_assertions requirement from 0.7.2 to 1.0.0; by dependabot[bot]; 2021-09-28

    Updates the requirements on [pretty_assertions](https://github.com/colin-kiegel/rust-pretty-assertions) to permit the latest version.
    - [Release notes](https://github.com/colin-kiegel/rust-pretty-assertions/releases)
    - [Changelog](https://github.com/colin-kiegel/rust-pretty-assertions/blob/main/CHANGELOG.md)
    - [Commits](https://github.com/colin-kiegel/rust-pretty-assertions/compare/v0.7.2...v1.0.0)

    ---
    updated-dependencies:
    - dependency-name: pretty_assertions
      dependency-type: direct:production
    ...

    Signed-off-by: dependabot[bot] <support@github.com>

-   Dependency: [cd080192](https://github.com/datafuselabs/openraft/commit/cd0801921a13c54949b41e2f408ddd54de4f13b6) update tracing-subscriber requirement from 0.2.10 to 0.3.3; by dependabot[bot]; 2021-11-30

    Updates the requirements on [tracing-subscriber](https://github.com/tokio-rs/tracing) to permit the latest version.
    - [Release notes](https://github.com/tokio-rs/tracing/releases)
    - [Commits](https://github.com/tokio-rs/tracing/compare/tracing-subscriber-0.2.10...tracing-subscriber-0.3.3)

    ---
    updated-dependencies:
    - dependency-name: tracing-subscriber
      dependency-type: direct:production
    ...

    Signed-off-by: dependabot[bot] <support@github.com>

### Added:

-   Added: [1451f962](https://github.com/datafuselabs/openraft/commit/1451f962dcc3e444a8e8dcc51ceb7d2c639411f8) Membership provides method is_majority() and simplify quorum calculation for voting; by drdr xp; 2021-12-25

-   Added: [a2a48c56](https://github.com/datafuselabs/openraft/commit/a2a48c569d69ec67feff7738f69aef6f8fda0b7a) make DefensiveCheck a reuseable trait; by drdr xp; 2021-12-26

    - Defensive checks in `MemStore` are moved out into a trait
      `DefensiveCheck`.

    - Let user impl a base `RaftStorage`. Then raft wraps it with a
      `StoreExt` thus the defensive checks apply to every impl of
      `RaftStorage`.

## v0.6.2-alpha.16

### Changed:

-   Changed: [79a39970](https://github.com/datafuselabs/openraft/commit/79a39970855d80e1d3b761fadbce140ecf1da59e) to get last_log and membership, Storage should search for both logs and state machines.; by drdr xp; 2021-08-24

    Why:

    depending on the impl, a RaftStore may have logs that are included in
    the state machine still present.
    This may be caused by a non-transactional impl of the store, e.g.
    installing snapshot and removing logs are not atomic.

    Thus when searching for last_log or last membership, a RaftStore should
    search for both logs and state machine, and returns the greater one
    that is found.

    - Test: add test to prove these behaviors,
      which includes: `get_initial_state()` and `get_membership()`.

    - Refactor: Make store tests a suite that could be applied to other
      impl.

-   Changed: [07d71c67](https://github.com/datafuselabs/openraft/commit/07d71c67a40766a302436f781294da931e1bc7d0) RaftStore::delete_logs_from() use range instead of (start, end); by drdr xp; 2021-08-28

-   Changed: [1c46a712](https://github.com/datafuselabs/openraft/commit/1c46a71241ad7ca6bcf69e35c27355a0ed185002) RaftStore::get_log_entries use range as arg; add try_get_log_entry() that does not return error even when defensive check is on; by drdr xp; 2021-08-28

### Added:

-   Added: [420cdd71](https://github.com/datafuselabs/openraft/commit/420cdd716b2ffa167d0dfcd0c2c21578793df88e) add defensive check to MemStore; by drdr xp; 2021-08-28

-   Added: [ab6689d9](https://github.com/datafuselabs/openraft/commit/ab6689d951954e3adbe8eb427364cf9062da1425) RaftStore::get_last_log_id() to get the last known log id in log or state machine; by drdr xp; 2021-08-29

### Fixed:

-   Fixed: [6d53aa12](https://github.com/datafuselabs/openraft/commit/6d53aa12f66ecd08e81bcb055eb17387b835e2eb) too many(50) inconsistent log should not live lock append-entries; by drdr xp; 2021-08-31

    - Reproduce the bug that when append-entries, if there are more than 50
      inconsistent logs,  the responded `conflict` is always set to
      `self.last_log`, which blocks replication for ever.
      Because the next time append-entries use the same `prev_log_id`, it
      actually does not search backward for the first consistent log entry.

      https://github.com/drmingdrmer/async-raft/blob/79a39970855d80e1d3b761fadbce140ecf1da59e/async-raft/src/core/append_entries.rs#L131-L154

    The test to reproduce it fakes a cluster of node 0,1,2:
    R0 has 100 uncommitted log at term 2.
    R2 has 100 uncommitted log at term 3.

    ```
    R0 ... 2,99 2,100
    R1
    R2 ... 3,99, 3,00
    ```

    Before this fix, brings up the cluster, R2 becomes leader and will never sync any log to R0.

    The fix is also quite simple:

    - Search backward instead of searching forward, to find the last log
      entry that matches `prev_log_id.term`, and responds this log id to the
      leader to let it send next `append_entries` RPC since this log id.

    - If no such matching term is found, use the first log id it sees, e.g.,
      the entry at index `prev_log_id.index - 50` for next `append_entries`.

-   Fixed: [9540c904](https://github.com/datafuselabs/openraft/commit/9540c904da4ae005baec01868e01016f3bc76810) when append-entries, deleting entries after prev-log-id causes committed entry to be lost; by drdr xp; 2021-08-31

    Problem:

    When append-entries, raft core removes old entries after
    `prev_log_id.index`, then append new logs sent from leader.

    Since deleting then appending entries are not atomic(two calls to `RaftStore`),
    deleting consistent entries may cause loss of committed entries, if
    server crashes after the delete.

    E.g., an example cluster state with logs as following and R1 now is the leader:

    ```
    R1 1,1  1,2  1,3
    R2 1,1  1,2
    R3
    ```

    Committed entry `{1,2}` gets lost after the following steps:

    - R1 to R2: `append_entries(entries=[{1,2}, {1,3}], prev_log_id={1,1})`
    - R2 deletes 1,2
    - R2 crash
    - R2 is elected as leader with R3, and only see 1,1; the committed entry 1,2 is lost.

    Solution:

    The safe way is to skip every entry that are consistent to the leader.
    And delete only the inconsistent entries.

    Another issue with this solution is that:

    Because we can not just delete `log[prev_log_id.index..]`, the commit index:
    - must be update only after append-entries,
    - and must point to a log entry that is consistent to leader.

    Or there could be chance applying an uncommitted entry:

    ```
    R0 1,1  1,2  3,3
    R1 1,1  1,2  2,3
    R2 1,1  1,2  3,3
    ```

    - R0 to R1 `append_entries: entries=[{1,2}], prev_log_id = {1,1}, commit_index = 3`
    - R1 accepted this `append_entries` request but was not aware of that entry {2,3} is inconsistent to leader.
      Updating commit index to 3 allows it to apply an uncommitted entries `{2,3}`.

## v0.6.2-alpha.15

### Changed:

-   Changed: [a1a05bb4](https://github.com/datafuselabs/openraft/commit/a1a05bb46eed4282af2c32d9db31f008b9519c15) rename Network methods to send_xxx; by drdr xp; 2021-08-23

-   Changed: [f168696b](https://github.com/datafuselabs/openraft/commit/f168696ba44c8af2a9106cfcde3ccb0d7c62d46a) rename RaftStorage::Snapshot to RaftStorage::SnapsthoData; by drdr xp; 2021-08-23

-   Changed: [fea63b2f](https://github.com/datafuselabs/openraft/commit/fea63b2fef7a0c21cf9d6080da296d32265cd0e0) rename CurrentSnapshotData to Snapshot; by drdr xp; 2021-08-23

-   Changed: [fabf3e74](https://github.com/datafuselabs/openraft/commit/fabf3e74642df00270212676f8ccf743dec8f0ce) rename RaftStorage::create_snapshot() to RaftStorage::begin_receiving_snapshot; by drdr xp; 2021-08-23

-   Changed: [90329fbf](https://github.com/datafuselabs/openraft/commit/90329fbf2bd0f5dfbe9260dda72c6a9383543942) RaftStorage: merge append_entry_to_log and replicate_to_log into one method append_to_log; by drdr xp; 2021-08-24

-   Changed: [daf2ed89](https://github.com/datafuselabs/openraft/commit/daf2ed8963074fe15123dca9bffc23bfcefb5159) RaftStorage: remove apply_entry_to_state_machine; by drdr xp; 2021-08-24

-   Changed: [a18b98f1](https://github.com/datafuselabs/openraft/commit/a18b98f1976abc75a7ae6cb0aa4d7d19317b6f6f) SnapshotPointer do not store any info.; by drdr xp; 2021-08-24

    Use SnapshotPointer to store membership is a bad idea.
    It brings in troubles proving the consistency, e.g.:

    - When concurrent `do_log_compaction()` is called(it is not
      possible for now, may be possible in future. The correctness proof
      involving multiple component is a nightmare.)

    - Proof of correctness of consistency between
      `StateMachine.last_membership` and `SnapshotPointer.membership` is
      complicated.

    What we need is actually:
    - At least one committed log is left in the log storage,
    - and info in the purged log must be present in the state machine, e.g.
      membership

-   Changed: [5d0c0b25](https://github.com/datafuselabs/openraft/commit/5d0c0b25e28443f65415ac2af5b27a6c65b67ce7) rename SnapshotPointer to PurgedMarker; by drdr xp; 2021-08-24

-   Changed: [72b02249](https://github.com/datafuselabs/openraft/commit/72b0224909850c1740d2fa8aed747a786074e0f3) rename replicate_to_state_machine to apply_to_state_machine; by drdr xp; 2021-08-24

## v0.6.2-alpha.14

### Fixed:

-   Fixed: [eee8e534](https://github.com/datafuselabs/openraft/commit/eee8e534e0b0b9abdb37dd94aeb64dc1affd3ef7) snapshot replication does not need to send a last 0 size chunk; by drdr xp; 2021-08-22

-   Fixed: [8cd24ba0](https://github.com/datafuselabs/openraft/commit/8cd24ba0f0212e94e61f21a7be0ce0806fcc66d5) RaftCore.entries_cache is inconsistent with storage. removed it.; by drdr xp; 2021-08-23

    - When leader changes, `entries_cache` is cleared.
      Thus there may be cached entries wont be applied to state machine.

    - When applying finished, the applied entries are not removed from the
      cache.
      Thus there could be entries being applied more than once.

## v0.6.2-alpha.13

### Fixed:

-   Fixed: [2eccb9e1](https://github.com/datafuselabs/openraft/commit/2eccb9e1f82f1bf71f6a2cf9ef6da7bf6232fa84) install snapshot req with offset GE 0 should not start a new session.; by drdr xp; 2021-08-22

    A install-snapshot always ends with a req with data len to be 0 and
    offset GE 0.
    If such a req is re-sent, e.g., when timeout, the receiver will try to
    install a snapshot with empty data, if it just finished the previous
    install snapshot req(`snapshot_state` is None) and do not reject a
    install snapshot req with offset GE 0.
    Which results in a `fatal storage error`, since the storage tries to
    decode an empty snapshot data.

    - feature: add config `install_snapshot_timeout`.

## v0.6.2-alpha.12



## v0.6.2-alpha.11



## v0.6.2-alpha.10

### Fixed:

-   Fixed: [beb0302b](https://github.com/datafuselabs/openraft/commit/beb0302b4fec0758062141e727bf1bbcfd4d4b98) leader should not commit when there is no replication to voters.; by drdr xp; 2021-08-18

    When there is no replication to voters but there are replications to
    non-voters, the leader did not check non-voters for a quorum but just
    commits a log at once.

    This cause the membership change log from a single node always commits.
    E.g. start node 0, and non-voter 1, 2; then `change_membership({0, 1, 2})`,
    It just commits the joint-log at once.
    But according to raft paper, it should await a quorum of {0} and a
    quorum of {0, 1, 2}.

### Changed:

-   Changed: [6350514c](https://github.com/datafuselabs/openraft/commit/6350514cc414dc9d7e9aa0e21ea7b546ed223235) change-membership should be log driven but not channel driven; by drdr xp; 2021-08-18

    A membership change involves two steps: the joint config phase and the
    final config phase.
    Each phase has a corresponding log involved.

    Previously the raft setup several channel to organize this workflow,
    which makes the logic hard to understand and introduces complexity when
    restarting or leadership transferred: it needs to re-establish the channels and tasks.

    According to the gist of raft, all workflow should be log driven.
    Thus the new approach:
    - Write two log(the joint and the final) at once it receives a
      change-membership request.
    - All following job is done according to just what log is committed.

    This simplifies the workflow and makes it more reliable and intuitive to
    understand.

    Related changes:

    - When `change_membership` is called, append 2 logs at once.

    - Introduce universal response channel type to send back a message when
      some internal task is done: `ResponseTx`, and a universal response
      error type: `ResponseError`.

    - Internal response channel is now an `Option<ResponseTx>`, since the
      first step of membership change does not need to respond to the
      caller.

    - When a new leaser established, if the **last** log is a joint config
      log, append a final config log to let the partial change-membership be
      able to complete.

      And the test is added.

    - Removed membership related channels.

    - Refactor: convert several func from async to sync.

## v0.6.2-alpha.9

### Changed:

-   Changed: [8b59966d](https://github.com/datafuselabs/openraft/commit/8b59966dd0a6bf804eb0ba978b5375010bfbc3f3) MembershipConfig.member type is changed form HashSet BTreeSet; by drdr xp; 2021-08-17

## v0.6.2-alpha.8

### Changed:

-   Changed: [adc24f55](https://github.com/datafuselabs/openraft/commit/adc24f55d75d9c7c01fcd0f4f9e35dd5aae679aa) pass all logs to apply_entry_to_state_machine(), not just Normal logs.; by drdr xp; 2021-08-16

    Pass `Entry<D>` to `apply_entry_to_state_machine()`, not just the only
    `EntryPayload::Normal(normal_log)`.

    Thus the state machine is able to save the membership changes if it
    prefers to.

    Why:

    In practice, a snapshot contains info about all applied logs, including
    the membership config log.
    Before this change, the state machine does not receive any membership
    log thus when making a snapshot, one needs to walk through all applied
    logs to get the last membership that is included in state machine.

    By letting the state machine remember the membership log applied,
    the snapshto creation becomes more convenient and intuitive: it does not
    need to scan the applied logs any more.

## v0.6.2-alpha.7



## v0.6.2-alpha.6

### Changed:

-   Changed: [82a3f2f9](https://github.com/datafuselabs/openraft/commit/82a3f2f9c7ac37a0f24c6e0e8993c8d3bcee5666) use LogId to track last applied instead of using just an index.; by drdr xp; 2021-07-19

    It provides more info by Using LogId to track last applied log.
    E.g. when creating a snapshot, it need to walk through logs to find the
    term of the last applied log, just like it did in memstore impl.

    Using LogId{term, index} is a more natural way in every aspect.

    changes: RaftCore: change type of `last_applied` from u64 to LogId.

## v0.6.2-alpha.5

### Fixed:

-   Fixed: [fc8e92a8](https://github.com/datafuselabs/openraft/commit/fc8e92a8207c1cf8bd1dba2e8de5c0c5eebedc1c) typo; by drdr xp; 2021-07-12

-   Fixed: [447dc11c](https://github.com/datafuselabs/openraft/commit/447dc11cab51fb3b1925177d13e4dd89f998837b) when finalize_snapshot_installation, memstore should not load membership from its old log that are going to be overridden by snapshot.; by drdr xp; 2021-07-13

-   Fixed: [dba24036](https://github.com/datafuselabs/openraft/commit/dba24036cda834e8c970d2561b1ff435afd93165) after 2 log compaction, membership should be able to be extract from prev compaction log; by drdr xp; 2021-07-14

### Changed:

-   Changed: [7792cccd](https://github.com/datafuselabs/openraft/commit/7792cccd229aa6a9248942fd40e6b40ee1570104) add CurrentSnapshotData.meta: SnapshotMeta, which is a container of all meta data of a snapshot: last log id included, membership etc.; by drdr xp; 2021-07-13

-   Changed: [0c870cc1](https://github.com/datafuselabs/openraft/commit/0c870cc1d4a49bbebca9f1b0c2a9ca56d015ea0e) reduce one unnecessary snapshot serialization; by drdr xp; 2021-07-14

    - Change: `get_current_snapshot()`: remove double-serialization:
      convert MemStoreSnapshot to CurrentSnapshotData instead of serializing
      MemStoreSnapshot:

      Before:
      ```
      MemStoreSnapshot.data = serialize(state-machine)
      CurrentSnapshotData.data = serialize(MemStoreSnapshot)
      ```

      After:
      ```
      MemStoreSnapshot.data = serialize(state-machine)
      CurrentSnapshotData.data = MemStoreSnapshot.data
      ```

      when `finalize_snapshot_installation`, extract snapshot meta info from
      `InstallSnapshotRequest`. Reduce one unnecessary deserialization.

    - Change: InstallSnapshotRequest: merge `snapshot_id`, `last_log_id`,
      `membership` into one field `meta`.

    - Refactor: use SnapshotMeta(`snapshot_id`, `last_log_id`, `membership`) as
      a container of metadata of a snapshot.
      Reduce parameters.

    - Refactor: remove redundant param `delete_through` from
      `finalize_snapshot_installation`.

## v0.6.2-alpha.4

### Changed:

-   Changed: [954c67a9](https://github.com/datafuselabs/openraft/commit/954c67a9dadadb5f8a02f192f1b83c7906ea1e85) InstallSnapshotRequest: merge last_included{term,index} into last_included; by drdr xp; 2021-07-08

-   Changed: [933e0b32](https://github.com/datafuselabs/openraft/commit/933e0b32822784957c5629f7d0ed3257b281f011) use snapshot-id to identify a snapshot stream; by drdr xp; 2021-07-09

    A snapshot stream should be identified by some id, since the server end
    should not assume messages are arrived in the correct order.
    Without an id, two `install_snapshot` request belonging to different
    snapshot data may corrupt the snapshot data, explicitly or even worse,
    silently.

    - Add SnapshotId to identify a snapshot stream.

    - Add SnapshotSegmentId to identify a segment in a snapshot stream.

    - Add field `snapshot_id` to snapshot related data structures.

    - Add error `RaftError::SnapshotMismatch`.

    - `Storage::create_snapshot()` does not need to return and id.
      Since the receiving end only keeps one snapshot stream session at
      most.
      Instead, `Storage::do_log_compaction()` should build a unique id
      everytime it is called.

    - When the raft node receives an `install_snapshot` request, the id must
      match to continue.
      A request with a different id should be rejected.
      A new id with offset=0 indicates the sender has started a new stream.
      In this case, the old unfinished stream is dropped and cleaned.

    - Add test for `install_snapshot` API.

-   Changed: [85859d07](https://github.com/datafuselabs/openraft/commit/85859d07d50468cec66ddec78ff3256d0f25b7f8) CurrentSnapshotData: merge `term` and `index` into `included`.; by drdr xp; 2021-07-09

-   Changed: [5eb9d3af](https://github.com/datafuselabs/openraft/commit/5eb9d3afd7a0b4fef13fd4b87b053bed09c91d9e) RaftCore: replace `snapshot_index` with `snapshot_last_included: LogId`. Keep tracks of both snapshot last log term and index.; by drdr xp; 2021-07-09

    Also `SnapshotUpdate::SnapshotComplete` now contains an LogId instead of an u64 index.

-   Changed: [9c5f3d7e](https://github.com/datafuselabs/openraft/commit/9c5f3d7e7049caeaef4c64ef0bfe9e1a6a8f4a62) RaftCore: merge last_log_{term,index} into last_log: LogId; by drdr xp; 2021-07-09

-   Changed: [58d8e3a2](https://github.com/datafuselabs/openraft/commit/58d8e3a2f23a0325a2800bea03cd797f45dec7bc) AppendEntriesRequest: merge prev_log_{term,index} into prev_log: LogId; by drdr xp; 2021-07-10

-   Changed: [9e4fb64f](https://github.com/datafuselabs/openraft/commit/9e4fb64f20c0fed68bbb665e76cc53f8385fb0d0) InitialState: last_log_{term,index} into last_log: LogId; by drdr xp; 2021-07-10

-   Changed: [24e38130](https://github.com/datafuselabs/openraft/commit/24e3813053bb2dafe3c3f1c65ce6ef462b79bb12) Entry: merge term and index to log_id: LogId; by drdr xp; 2021-07-11

### Added:

-   Added: [8e0b0df9](https://github.com/datafuselabs/openraft/commit/8e0b0df9f00787feb8315e9dae14aca94494a50f) report snapshot metrics to RaftMetrics::snapshot, which is a LogId: (term, index) that a snapshot includes; by drdr xp; 2021-07-09

    - Add: `Wait.snapshot()` to watch snapshot changes.
    - Test: replace `sleep()` with `wait_for_snapshot()` to speed up tests.

## v0.6.2-alpha.3

### Dependency:

-   Dependency: [b351c87f](https://github.com/datafuselabs/openraft/commit/b351c87f0adfd0a6f1105f55cf1223c6045ecf41) upgrade tokio from 1.7 to 1.8; by drdr xp; 2021-07-08

### Fixed:

-   Fixed: [cf4badd0](https://github.com/datafuselabs/openraft/commit/cf4badd0d762757519e2db5ed2f2fc65c2f49d02) leader should re-create and send snapshot when `threshold/2 < last_log_index - snapshot < threshold`; by drdr xp; 2021-07-08

    The problem:

    If `last_log_index` advances `snapshot.applied_index` too many, i.e.:
    `threshold/2 < last_log_index - snapshot < threshold`
    (e.g., `10/2 < 16-10 < 20` in the test that reproduce this bug), the leader
    tries to re-create a new snapshot. But when
    `last_log_index < threshold`, it won't create, which result in a dead
    loop.

    Solution:

    In such case, force to create a snapshot without considering the
    threshold.

## v0.6.2-alpha.2

### Dependency:

-   Dependency: [70e1773e](https://github.com/datafuselabs/openraft/commit/70e1773edcf5e2bc7369c7afe47bb1348bc2a274) adapt to changes of rand-0.8: gen_range() accepts a range instead of two args; by drdr xp; 2021-06-21

### Added:

-   Added: [32a67e22](https://github.com/datafuselabs/openraft/commit/32a67e228cf26f9207593805a0386cf6aa4fe294) add metrics about leader; by drdr xp; 2021-06-29

    In LeaderState it also report metrics about the replication to other node when report metrics.

    When switched to other state, LeaderState will be destroyed as long as
    the cached replication metrics.

    Other state report an `None` to raft core to override the previous
    metrics data.

    At some point the raft core, without knonwning the state, just report
    metrics with an `Update::Ignore`, to indicate that leave replication
    metrics intact.

### Fixed:

-   Fixed: [d60f1e85](https://github.com/datafuselabs/openraft/commit/d60f1e852d3e5b9455589593067599d261f695b2) client_read has using wrong quorum=majority-1; by drdr xp; 2021-07-02

## v0.6.2-alpha.1

### Added:

-   Added: [1ad17e8e](https://github.com/datafuselabs/openraft/commit/1ad17e8edf18d98eeb687f00a65ebf528ab3aeb7) move wait_for_xxx util into metrics.; by drdr xp; 2021-06-16

    Introduce struct `Wait` as a wrapper of the metrics channel to impl
    wait-for utils:
    - `log()`:  wait for log to apply.
    - `current_leader()`: wait for known leader.
    - `state()`: wait for the role.
    - `members()`: wait for membership_config.members.
    - `next_members()`: wait for membership_config.members_after_consensus.

    E.g.:

    ```rust
    // wait for ever for raft node's current leader to become 3:
    r.wait(None).current_leader(2).await?;
    ```

    The timeout is now an option arg to all wait_for_xxx functions in
    fixtures. wait_for_xxx_timeout are all removed.

## v0.6.2-alpha

### Added:

-   Added: [3388f1a2](https://github.com/datafuselabs/openraft/commit/3388f1a2757bfb8a1208c2e7e175946ef74db5e2) link to discord server.; by Anthony Dodd; 2021-05-21

-   Added: [bcc246cc](https://github.com/datafuselabs/openraft/commit/bcc246ccf8fa2858baf8e07bf760ee6db4d85cf6) a pull request template.; by Anthony Dodd; 2021-05-26

-   Added: [ea539069](https://github.com/datafuselabs/openraft/commit/ea53906997c668cee06f17516a710f29bd1c2c63) wait_for_nodes_log(); by drdr xp; 2021-05-24

-   Added: [668ad478](https://github.com/datafuselabs/openraft/commit/668ad478f7c567218d148f0c20c7611ae3ac927a) some wait_for func:; by drdr xp; 2021-05-24

    - wait_for_log()
    - wait_for_log_timeout()
    - wait_for_state()
    - wait_for_state_timeout()

### Fixed:

-   Fixed: [89bb48f8](https://github.com/datafuselabs/openraft/commit/89bb48f8702762d33b59a7c9b9710bde4a97478c) last_applied should be updated only when logs actually applied.; by drdr xp; 2021-05-20

-   Fixed: [e9f40450](https://github.com/datafuselabs/openraft/commit/e9f4045097b4c7ce4385b3a9a7a990d31af94d15) usage of get_storage_handle; by drdr xp; 2021-05-23

-   Fixed: [22cd1a0c](https://github.com/datafuselabs/openraft/commit/22cd1a0c2180edfb53464eec2bccc54778aff46c) clippy complains; by drdr xp; 2021-05-23

-   Fixed: [6202138f](https://github.com/datafuselabs/openraft/commit/6202138f0766dcfd07a1a825af165f132de6b920) a conflict is expected even when appending empty enties; by drdr xp; 2021-05-24

-   Fixed: [f449b64a](https://github.com/datafuselabs/openraft/commit/f449b64aa9254d9a18bc2abb5f602913af079ca9) discarded log in replication_buffer should be finally sent.; by drdr xp; 2021-05-22

    Internally when replication goes to LaggingState(a non-leader lacks a lot logs), the
    ReplicationCore purges `outbound_buffer` and `replication_buffer` and then sends all
    **committed** logs found in storage.

    Thus if there are uncommitted logs in `replication_buffer`, these log will never have chance to
    be replicated, even when replication goes back to LineRateState.
    Since LineRateState only replicates logs from `ReplicationCore.outbound_buffer` and
    `ReplicationCore.replication_buffer`.

    This test ensures that when replication goes to LineRateState, it tries to re-send all logs
    found in storage(including those that are removed from the two buffers.

-   Fixed: [6d680484](https://github.com/datafuselabs/openraft/commit/6d680484ee3e352d4caf37f5cd6f57630f46d9e2) #112 : when a follower is removed, leader should stops sending log to it.; by drdr xp; 2021-05-21

    A leader adds all follower replication states to a hashset `nodes`, when
    the leader is established.
    But the leader does not do it when membership changed.
    Thus when a follower is removed, the leader can not stop replication to
    it because the follower is not in `nodes`.

    The solution is to move replication state from `non_voters` to `nodes`.
    So that next time a follower is removed the leader is able to remove the
    replication from `nodes`.

-   Fixed: [39690593](https://github.com/datafuselabs/openraft/commit/39690593a07c6b9ded4b7b8f1aca3191fa7641e4) a NonVoter should stay as NonVoter instead of Follower after restart; by drdr xp; 2021-05-14

-   Fixed: [d882e743](https://github.com/datafuselabs/openraft/commit/d882e743db4734b2188b137ebf20c0443cf9fb49) when calc quorum, the non-voter should be count; by drdr xp; 2021-06-02

    Counting only the follower(nodes) as quorum for new config(c1) results
    in unexpected log commit.
    E.g.: change from 012 to 234, when 3 and 4 are unreachable, the first
    log of joint should not be committed.

-   Fixed: [a10d9906](https://github.com/datafuselabs/openraft/commit/a10d99066b8c447d7335c7f34e08bd78c4b49f61) when handle_update_match_index(), non-voter should also be considered, because when member change a non-voter is also count as a quorum member; by drdr xp; 2021-06-16

-   Fixed: [11cb5453](https://github.com/datafuselabs/openraft/commit/11cb5453e2200eda06d26396620eefe66b169975) doc-include can only be used in nightly build; by drdr xp; 2021-06-16

    - Simplify CI test: test all in one action.

    - Disable clippy: it suggests inappropriate assert_eq to assert
      conversion which is used in a macro.

    - Add makefile

    - Build only with nightly rust. Add rust-toolchain to specify toolchain
      version.

### Dependency:

-   Dependency: [919d91cb](https://github.com/datafuselabs/openraft/commit/919d91cb31b307cede7d0911ff45e1030174a340) upgrade tokio from 1.0 to 1.7; by drdr xp; 2021-06-16

## v0.6.1

## async-raft 0.6.1
### fixed
- Fixed [#105](https://github.com/async-raft/async-raft/issues/105) where function `set_target_state` missing `else` condition.
- Fixed [#106](https://github.com/async-raft/async-raft/issues/106) which ensures that counting of replicas to determine a new commit value only considers entries replicated as part of the current term.
- Fixed a bug where Learner nodes could be restarted and come back as voting members.

## async-raft 0.6.0
The big news for this release is that we are now based on Tokio 1.0! Big shoutout to @xu-cheng for doing all of the heavy lifting for the Tokio 1.0 update, along with many other changes which are part of this release.

It is important to note that 0.6.0 does include two breaking changes from 0.5: the new `RaftStorage::ShutdownError` associated type, and Tokio 1.0. Both of these changes are purely code related, and it is not expected that they will negatively impact running systems.

### changed
- Updated to Tokio 1.0!
- **BREAKING:** this introduces a `RaftStorage::ShutdownError` associated type. This allows for the Raft system to differentiate between fatal storage errors which should cause the system to shutdown vs errors which should be propagated back to the client for application specific error handling. These changes only apply to the `RaftStorage::apply_entry_to_state_machine` method.
- A small change to Raft startup semantics. When a node comes online and successfully recovers state (the node was already part of a cluster), the node will start with a 30 second election timeout, ensuring that it does not disrupt a running cluster.
- [#89](https://github.com/async-raft/async-raft/pull/89) removes the `Debug` bounds requirement on the `AppData` & `AppDataResponse` types.
- The `Raft` type can now be cloned. The clone is very cheap and helps to facilitate async workflows while feeding client requests and Raft RPCs into the Raft instance.
- The `Raft.shutdown` interface has been changed slightly. Instead of returning a `JoinHandle`, the method is now async and simply returns a result.
- The `ClientWriteError::ForwardToLeader` error variant has been modified slightly. It now exposes the data (generic type `D` of the type) of the original client request directly. This ensures that the data can actually be used for forwarding, if that is what the parent app wants to do.
- Implemented [#12](https://github.com/async-raft/async-raft/issues/12). This is a pretty old issue and a pretty solid optimization. The previous implementation of this algorithm would go to storage (typically disk) for every process of replicating entries to the state machine. Now, we are caching entries as they come in from the leader, and using only the cache as the source of data. There are a few simple measures needed to ensure this is correct, as the leader entry replication protocol takes care of most of the work for us in this case.
- Updated / clarified the interface for log compaction. See the guide or the updated `do_log_compaction` method docs for more details.

### added
- [#97](https://github.com/async-raft/async-raft/issues/97) adds the new `Raft.current_leader` method. This is a convenience method which builds upon the Raft metrics system to quickly and easily identify the current cluster leader.

### fixed
- Fixed [#98](https://github.com/async-raft/async-raft/issues/98) where heartbeats were being passed along into the log consistency check algorithm. This had the potential to cause a Raft node to go into shutdown under some circumstances.
- Fixed a bug where the timestamp of the last received heartbeat from a leader was not being stored, resulting in degraded cluster stability under some circumstances.

## memstore 0.2.0
### changed
- Updated async-raft dependency to `0.6.0` & updated storage interface as needed.

### fixed
- Fixed [#76](https://github.com/async-raft/async-raft/issues/76) by moving the process of replicating log entries to the state machine off of the main task. This ensures that the process never blocks the main task. This also includes a few nice optimizations mentioned below.

## 0.5.5
### changed
- Added `#[derive(Serialize, Deserialize)]` to `RaftMetrics`, `State`.

## 0.5.4
### fixed
- Fixed [#82](https://github.com/async-raft/async-raft/issues/82) where client reads were not behaving correctly for single node clusters. Single node integration tests have been updated to ensure this functionality is working as needed.

## 0.5.3
### fixed
- Fixed [#79](https://github.com/async-raft/async-raft/issues/79) ... for real this time! Add an integration test to prove it.

## 0.5.2
### fixed
- Fixed [#79](https://github.com/async-raft/async-raft/issues/79). The Raft core state machine was not being properly updated in response to shutdown requests. That has been addressed and shutdowns are now behaving as expected.

## 0.5.1
### changed
- `ChangeMembershipError::NodeNotLeader` now returns the ID of the current cluster leader if known.
- Fix off-by-one error in `get_log_entries` during the replication process.
- Added `#[derive(Serialize, Deserialize)]` to `Config`, `ConfigBuilder` & `SnapshotPolicy`.

## 0.5.0
### changed
The only thing which hasn't changed is that this crate is still an implementation of the Raft protocol. Pretty much everything else has changed.

- Everything is built directly on Tokio now.
- The guide has been updated.
- Docs have been updated.
- The `Raft` type is now the primary API of this crate, and is a simple struct with a few public methods.
- Lots of fixes to the implementation of the protocol, ranging from subtle issues in joint consensus to non-voter syncing.

## 0.4.4
- Implemented `Error` for `config::ConfigError`

## 0.4.3
Added a few convenience derivations.

- Derive `Eq` on `messages::MembershipConfig`.
- Derive `Eq` on `metrics::State`.
- Derive `PartialEq` & `Eq` on `metrics::RaftMetrics`.
- Update development dependencies.
- Fixed bug [#41](https://github.com/railgun-rs/actix-raft/issues/41) where nodes were not starting a new election timeout task after comign down from leader state. Thanks @lionesswardrobe for the report!

## 0.4.2
A few QOL improvements.

- Fixed an issue where the value for `current_leader` was not being set to `None` when becoming a candidate. This isn't really a *bug* per se, as no functionality depended on this value as far as Raft is concerned, but it is an issue that impacts the metrics system. This value is now being updated properly.
- Made the `messages::ClientPayload::new_base` constructor `pub(crate)` instead of `pub`, which is what the intention was originally, but I was apparently tired `:)`.
- Implemented [#25](https://github.com/railgun-rs/actix-raft/issues/25). Implementing Display+Error for the admin error types.

## 0.4.1
A few bug fixes.

- Fixed an issue where a node in a single-node Raft was not resuming as leader after a crash.
- Fixed an issue where hard state was not being saved after a node becomes leader in a single-node Raft.
- Fixed an issue where the client request pipeline (a `Stream` with the `actix::StreamFinish`) was being closed after an error was returned during processing of client requests (which should not cause the stream to close). This was unexpected and undocumented behavior, very simple fix though.

## 0.4.0
This changeset introduces a new `AppDataResponse` type which represents a concrete data type which must be sent back from the `RaftStorage` impl from the `ApplyEntryToStateMachine` handler. This provides a more direct path for returning application level data from the storage impl. Often times this is needed for responding to client requests in a timely / efficient manner.

- `AppDataResponse` type has been added (see above).
- A few handlers have been updated in the `RaftStorage` type. The handlers are now separated based on where they are invoked from the Raft node. The three changed handlers are:
  - `AppendEntryToLog`: this is the same. It is the initial step of handling client requests to apply an entry to the log. This is still where application level errors may be safely returned to the client.
  - `ReplicateToLog`: this is for replicating entries to the log. This is part of the replication process.
  - `ApplyEntryToStateMachine`: this is for applying an entry to the state machine as the final part of a client request. This is where the new `AddDataResponse` type must be returned.
  - `ReplicateToStateMachine`: this is for replicating entries to the state machine. This is part of the replication process.

## 0.3.1
Overhauled the election timeout mechanism. This uses an interval job instead of juggling a rescheduling processes. Seems to offer quite a lot more stability. Along with the interval job, we are using std::time::Instants for performing the comparisons against the last received heartbeat.

## 0.3.0
Another backwards incompatible change to the `RaftStorage` trait. It is now using associated types to better express the needed trait constraints. These changes were the final bit of work needed to get the entire actix-raft system to work with a Synchronous `RaftStorage` impl. Async impls continue to work as they have, the `RaftStorage` impl block will need to be updated to use the associated types though. The recommend pattern is as follows:

```rust
impl RaftStorage<..., ...> for MyStorage {
    type Actor = Self;
    type Context = Context<Self>; // Or SyncContext<Self>;
}
```

My hope is that this will be the last backwards incompatible change needed before a 1.0 release. This crate is still young though, so we will see.

## 0.2.0
- Made a few backwards incompatible changes to the `RaftStorage` trait. Overwrite its third type parameter with `actix::SyncContext<Self>` to enable sync storage.
- Also removed the `RaftStorage::new` constructor, as it is a bit restrictive. Just added some docs instead describing what is needed.

## 0.1.3
- Added a few addition top-level exports for convenience.

## 0.1.2
- Changes to the README for docs.rs.

## 0.1.1
- Changes to the README for docs.rs.

## 0.1.0
- Initial release!

