## v0.6.9

### Dependency:

-   Dependency: [a97a3097](https://github.com/datafuselabs/openraft/commit/a97a3097b56078e3dc20aa29df1fff07090792cd) adapt rust stable-1.63 and nightly 2022-08-11; by 张炎泼; 2022-09-02

    - `std::backtrace` is stabilized in stable-1.63;
    - `std::Error::backtrace()` is replaced with provider style API in
      nightly.

    Since `std::Error` API now is different in stable and nightly, it's not
    possible any more to use `--feature bt` with stable toolchain.

### Fixed:

-   Fixed: [d5b52d35](https://github.com/datafuselabs/openraft/commit/d5b52d35d35643a6039b75039e81c78298b74758) snapshot with smaller last_log_id than last_applied should not be installed; by 张炎泼; 2022-08-25

    Otherwise, the state machine will revert to an older state, while the
    in-memory `last_applied` is unchanged.
    This finally causes logs that falls in range `(snapshot.last_log_id, last_applied]` can not be applied.

## v0.6.8

### Fixed:

-   Fixed: [485dbe9a](https://github.com/datafuselabs/openraft/commit/485dbe9a783de05437c665eca27541a5acd4d36b) when handling append-entries, if prev_log_id is purged, it should not delete any logs.; by 张炎泼; 2022-08-14

## v0.6.7

### Added:

-   Added: [e9d772be](https://github.com/datafuselabs/openraft/commit/e9d772bed84210c834d01fbeae15e999ec195ef6) add feature-flag: `bt` enables backtrace; by 张炎泼; 2022-03-12

    `--features bt` enables backtrace when generating errors.
    By default errors does not contain backtrace info.

    Thus openraft can be built on stable rust by default.

    To use on stable rust with backtrace, set `RUSTC_BOOTSTRAP=1`, e.g.:
    ```
    RUSTUP_TOOLCHAIN=stable RUSTC_BOOTSTRAP=1 make test
    ```

## v0.6.6

### Changed:

-   Changed: [af10f087](https://github.com/datafuselabs/openraft/commit/af10f087406d25b0a1da7fe8605f47f78837b2f4) API: use AnyError and string backtrace in StorageIOError and Violation.; by 张炎泼; 2022-07-04

## v0.6.5

### Fixed:

-   Fixed: [4cd2a12b](https://github.com/datafuselabs/openraft/commit/4cd2a12b09c5b574e79c81f39a301597f1d4bd6b) span.enter() in async loop causes memory leak; by 张炎泼; 2022-06-17

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
      Updating commit index to 3 allows it to apply an uncommitted entrie `{2,3}`.

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
    Each phase has a corresponding log invovled.

    Previously the raft setup several channel to organize this workflow,
    which makes the logic hard to understand and introduces complexity when
    restarting or leadership transfered: it needs to re-establish the channels and tasks.

    According to the gist of raft, all workflow should be log driven.
    Thus the new approach:
    - Write two log(the joint and the final) at once it recevies a
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
    the snapshto creation becomes more convinient and intuitive: it does not
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

    - Refactor: remove redundent param `delete_through` from
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
      convertion which is used in a macro.

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

