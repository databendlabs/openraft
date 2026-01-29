# OpenRaft TODO Priority List

**Analysis Date:** 2026-01-29  
**Total TODOs Found:** 70

This document provides a comprehensive list of all TODO items found in the OpenRaft codebase, organized by priority.

---

## Executive Summary

- **High Priority (Critical):** 5 items - API changes, error handling, performance issues
- **Medium Priority (Improvements):** 57 items - Feature enhancements, tests, documentation
- **Low Priority (Cleanup):** 8 items - Code cleanup, removal of unused code

---

## Top 10 TODO Items (Highest Priority)

These are the most critical TODO items that should be addressed first, based on their impact on API stability, correctness, and performance:

### 1. API Change: Replace JoinError with Fatal
**File:** `openraft/src/raft/mod.rs:1825`  
**Priority:** CRITICAL - API Breaking Change  
**Description:** API change needed to replace `JoinError` with `Fatal` error type.  
**Impact:** Public API change that affects error handling across the codebase.

### 2. Reduce StorageError Size
**File:** `openraft/src/lib.rs:7`  
**Priority:** HIGH - Performance  
**Description:** `clippy::result-large-err`: StorageError is 136 bytes, try to reduce the size.  
**Impact:** Performance optimization for error handling, reducing memory overhead.

### 3. Fix unwrap() Usage
**File:** `examples/raft-kv-memstore-grpc/src/pb_impl/impl_membership.rs:17`  
**Priority:** HIGH - Code Safety  
**Description:** Remove unwrap() calls to prevent potential panics.  
**Impact:** Improves error handling and prevents runtime panics.

### 4. ClientWriteError Type Constraint
**File:** `openraft/src/raft/api/app.rs:58`  
**Priority:** HIGH - Error Handling  
**Description:** ClientWriteError can only be ForwardToLeader Error.  
**Impact:** Clarifies error handling contract in the API.

### 5. Membership Config Error Handling
**File:** `openraft/src/core/raft_core.rs:615`  
**Priority:** HIGH - Error Handling  
**Description:** Should return membership config error properly.  
**Impact:** Improves error reporting for membership operations.

### 6. Make Engine Fields Private
**File:** `openraft/src/engine/engine_impl.rs:67`  
**Priority:** MEDIUM-HIGH - API Design  
**Description:** Make the engine implementation fields private for better encapsulation.  
**Impact:** Improves API design and encapsulation.

### 7. Optimize is_voter() Performance
**File:** `openraft/src/core/raft_core.rs:905`  
**Priority:** MEDIUM-HIGH - Performance  
**Description:** `is_voter()` is slow, maybe cache `current_leader`.  
**Impact:** Performance optimization for frequently called function.

### 8. Make Method Non-Async
**File:** `openraft/src/core/raft_core.rs:1488`  
**Priority:** MEDIUM-HIGH - Performance  
**Description:** Make this method non-async as it doesn't need async operations.  
**Impact:** Performance improvement and cleaner API.

### 9. Store Node ID in RaftStore
**File:** `openraft/src/storage/helper.rs:79`  
**Priority:** MEDIUM - Storage Design  
**Description:** Let RaftStore store node-id for better state management.  
**Impact:** Improves storage design and state consistency.

### 10. Add Configurable Settings
**File:** `openraft/src/core/raft_core.rs:1245`  
**Priority:** MEDIUM - Configurability  
**Description:** Make hardcoded values configurable.  
**Impact:** Improves flexibility and allows better tuning.

---

## All High Priority TODO Items (5 items)

### 1. Fix unwrap() Usage
**File:** `examples/raft-kv-memstore-grpc/src/pb_impl/impl_membership.rs:17`  
**Category:** Code Safety  
```rust
// TODO: do not unwrap()
```

### 2. ClientWriteError Type Constraint
**File:** `openraft/src/raft/api/app.rs:58`  
**Category:** Error Handling  
```rust
// TODO: ClientWriteError can only be ForwardToLeader Error
```

### 3. API Change: Replace JoinError with Fatal
**File:** `openraft/src/raft/mod.rs:1825`  
**Category:** API Breaking Change  
```rust
// TODO(xp): API change: replace `JoinError` with `Fatal`,
```

### 4. Reduce StorageError Size
**File:** `openraft/src/lib.rs:7`  
**Category:** Performance  
```rust
// TODO: `clippy::result-large-err`: StorageError is 136 bytes, try to reduce the size.
```

### 5. Membership Config Error Handling
**File:** `openraft/src/core/raft_core.rs:615`  
**Category:** Error Handling  
```rust
// TODO: it should returns membership config error etc. currently this is done by the
```

---

## Medium Priority TODO Items (57 items)

### Core Engine & State Management (12 items)

1. **Enable Documentation** - `openraft/src/lib.rs:12`
   ```rust
   // TODO: Enable this when doc is complete
   ```

2. **Make Engine Fields Private** - `openraft/src/engine/engine_impl.rs:67`
   ```rust
   /// TODO: make the fields private
   ```

3. **Add Voting State Container** - `openraft/src/engine/engine_impl.rs:77`
   ```rust
   // TODO: add a Voting state as a container.
   ```

4. **Refactor Internal Server State Update** - `openraft/src/engine/engine_impl.rs:153`
   ```rust
   // TODO: replace all the following codes with one update_internal_server_state;
   ```

5. **Improve Vote Handling** - `openraft/src/engine/tests/handle_vote_resp_test.rs:100`
   ```rust
   // TODO: when seeing a higher vote, keep trying until a majority of higher votes are seen.
   ```

6. **Optimize Committed Write** - `openraft/src/engine/command.rs:293`
   ```rust
   // TODO: Apply also write `committed` to log-store, which should be run in CommandKind::Log
   ```

7. **Leader Lease Decision** - `openraft/src/engine/handler/vote_handler/mod.rs:118`
   ```rust
   // TODO: Leader decide the lease.
   ```

8. **Handle Edge Case** - `openraft/src/engine/handler/vote_handler/mod.rs:185`
   ```rust
   // TODO: this is not gonna happen,
   ```

9. **Optimize Following State** - `openraft/src/engine/handler/vote_handler/mod.rs:231`
   ```rust
   // TODO: if it is already in following state, nothing to do.
   ```

10. **Update Replication on Membership Change** - `openraft/src/engine/handler/following_handler/mod.rs:214`
    ```rust
    // TODO: if effective membership changes, call `update_replication()`, if a follower has replication
    ```

11. **Snapshot Update Logic** - `openraft/src/engine/handler/following_handler/install_snapshot_test.rs:93`
    ```rust
    // `committed`. TODO: The snapshot should be able to be updated if
    ```

12. **Vote Validation for Leader** - `openraft/src/engine/handler/leader_handler/mod.rs:53`
    ```rust
    /// TODO(xp): if vote indicates this node is not the leader, refuse append
    ```

### Replication Handler (7 items)

13. **Expand Replication Setup** - `openraft/src/engine/handler/replication_handler/mod.rs:77`
    ```rust
    // TODO(9): currently only a leader has replication setup.
    ```

14. **Add Test Coverage** - `openraft/src/engine/handler/replication_handler/mod.rs:232`
    ```rust
    // TODO(2): test it?
    ```

15. **Update Matching for Leader** - `openraft/src/engine/handler/replication_handler/mod.rs:349`
    ```rust
    // TODO: update matching should be done here for leader
    ```

16. **Refactor Replication Code** - `openraft/src/engine/handler/replication_handler/mod.rs:412`
    ```rust
    // TODO refactor this
    ```

17. **Add Test** - `openraft/src/engine/handler/replication_handler/mod.rs:413`
    ```rust
    // TODO: test
    ```

18. **Unify Replication API** - `openraft/src/engine/handler/replication_handler/mod.rs:447`
    ```rust
    // TODO: replication handler should provide the same API for both locally and remotely log
    ```

19. **Fix Log ID Reference** - `openraft/src/engine/handler/replication_handler/mod.rs:469`
    ```rust
    // TODO: It should be self.state.last_log_id() but None is ok.
    ```

### Storage & State Machine (7 items)

20. **Make State Machine Sync** - `openraft/src/storage/v2/raft_state_machine.rs:36`
    ```rust
    // TODO: This can be made into sync, provided all state machines will use atomic read or the
    ```

21. **Complete Vote Change Test** - `openraft/src/storage/v2/raft_log_reader.rs:107`
    ```rust
    // TODO: complete the test that ensures when the vote is changed, stream should be stopped.
    ```

22. **Store Node ID** - `openraft/src/storage/helper.rs:79`
    ```rust
    // TODO: let RaftStore store node-id.
    ```

23. **Handle Committed vs Last Applied** - `openraft/src/storage/helper.rs:113`
    ```rust
    // TODO: It is possible `committed < last_applied` because when installing snapshot,
    ```

24. **Handle Lease Revert** - `openraft/src/storage/helper.rs:208`
    ```rust
    // TODO: If the lease reverted upon restart,
    ```

25. **Store Purged Separately** - `openraft/src/storage/helper.rs:404`
    ```rust
    // TODO: store purged: Option<LogId> separately.
    ```

26. **Move Documentation** - `openraft/src/raft_state/mod.rs:370`
    ```rust
    // TODO: move these doc to the [`IOState`]
    ```

### Core Raft Logic (15 items)

27. **Read Request Optimization** - `openraft/src/core/raft_core.rs:315`
    ```rust
    // TODO: the second condition is such a read request can only read from state machine only when the last log it sees
    ```

28. **Applied State Staleness** - `openraft/src/core/raft_core.rs:333`
    ```rust
    // TODO: this applied is a little stale when being returned to client.
    ```

29. **Manage Read Requests with Queue** - `openraft/src/core/raft_core.rs:509`
    ```rust
    // TODO: do not spawn, manage read requests with a queue by RaftCore
    ```

30. **Change Membership Leadership Check** - `openraft/src/core/raft_core.rs:832`
    ```rust
    // TODO: change-membership should check leadership or wait for leader to establish?
    ```

31. **Cache Current Leader** - `openraft/src/core/raft_core.rs:905`
    ```rust
    // TODO: `is_voter()` is slow, maybe cache `current_leader`,
    ```

32. **Make Configurable** - `openraft/src/core/raft_core.rs:1245`
    ```rust
    // TODO: make it configurable
    ```

33. **Optimize Engine Commands** - `openraft/src/core/raft_core.rs:1312`
    ```rust
    // TODO: does run_engine_commands() run too frequently?
    ```

34. **Make Method Non-Async** - `openraft/src/core/raft_core.rs:1488`
    ```rust
    // TODO: Make this method non-async. It does not need to run any async command in it.
    ```

35. **Reject Already Leader** - `openraft/src/core/raft_core.rs:1581`
    ```rust
    // TODO: reject if it is already a leader?
    ```

36. **Test Fixture Improvement** - `openraft/src/core/raft_core.rs:1698`
    ```rust
    // TODO: test: fixture: make isolated_nodes a single-way isolating.
    ```

37. **Replace Temp Solution** - `openraft/src/core/raft_core.rs:1728`
    ```rust
    // TODO: temp solution: Manually wait until the second membership log being applied to state
    ```

38. **Extend Leader Lease** - `openraft/src/core/raft_core.rs:1830`
    ```rust
    // TODO: leader lease should be extended. Or it has to examine if it is leader
    ```

39. **Clean Snapshot Transmission** - `openraft/src/core/raft_core.rs:2182`
    ```rust
    // TODO: it is not cleaned when snapshot transmission is done.
    ```

40. **Make Message Receiver Configurable** - `openraft/src/core/merged_raft_msg_receiver.rs:158`
    ```rust
    // TODO: make it configurable
    ```

41. **Abortable Worker** - `openraft/src/core/sm/worker.rs:251`
    ```rust
    // TODO: need to be abortable?
    ```

### Other Components (16 items)

42. **Metrics TODO** - `metrics-otel/TODO.md:1`
    ```
    # Future Metrics TODO
    ```

43. **Add Diagram to Guides** - `openraft/src/runtime/mod.rs:59`
    ```rust
    /// TODO: add this diagram to guides/
    ```

44. **Handle Leader ID Difference** - `openraft/src/proposer/candidate.rs:120`
    ```rust
    // TODO: tricky: the new LeaderId is different from the last log id
    ```

45. **Generalize Log ID Range** - `openraft/src/log_id_range.rs:12`
    ```rust
    // TODO: I need just a range, but not a log id range.
    ```

46. **Refactor Snapshot Transmitter** - `openraft/src/replication/snapshot_transmitter.rs:79`
    ```rust
    // TODO: this function should just return join_handle and let the caller build
    ```

47. **Merge Get Methods** - `openraft/src/progress/mod.rs:79`
    ```rust
    // TODO: merge `get` and `try_get`
    ```

48. **Setup Log IDs in Test** - `openraft/src/progress/entry/tests.rs:105`
    ```rust
    // TODO: setup log_ids
    ```

49. **Add More Tests** - `openraft/src/engine/handler/server_state_handler/update_server_state_test.rs:60`
    ```rust
    // TODO(3): add more test,
    ```

50. **Test Log Compaction** - `openraft/src/testing/log/suite.rs:179`
    ```rust
    // TODO(xp): test: do_log_compaction
    ```

51. **Test Normal Entry Application** - `openraft/src/testing/log/suite.rs:1596`
    ```rust
    // TODO: figure out how to test applying normal entry. `C::D` cannot be built by Openraft
    ```

### Test Suite TODOs (7 items)

52. **Auto Commit Discussion** - `tests/tests/membership/t99_new_leader_auto_commit_uniform_config.rs:17`
    ```rust
    /// TODO(xp): in discussion: whether a leader should auto commit a uniform membership config:
    ```

53. **Fix Flaky Test** - `tests/tests/membership/t11_add_learner.rs:260`
    ```rust
    // TODO(1): flaky with --features single-term-leader
    ```

54. **Leader Stability Issue** - `tests/tests/membership/t21_change_membership_cases.rs:231`
    ```rust
    // TODO(xp): leader may not be stable, other node may take leadership by a higher vote.
    ```

55. **Leader Stability Issue (2)** - `tests/tests/membership/t21_change_membership_cases.rs:439`
    ```rust
    // TODO(xp): leader may not be stable, other node may take leadership by a higher vote.
    ```

56. **Snapshot Replication Issue** - `tests/tests/append_entries/t90_issue_216_stale_last_log_id.rs:19`
    ```rust
    /// TODO(xp): `max_applied_log_to_keep` to be 0 makes it very easy to enter snapshot replication and
    ```

57. **Verify Snapshot Last Log ID** - `tests/tests/snapshot_building/t11_snapshot_builder_control.rs:86`
    ```rust
    // TODO: verify io_snapshot_last_log_id is None; require public method to access it.
    ```

---

## Low Priority TODO Items (Code Cleanup - 8 items)

### 1. Remove Serde Dependency
**File:** `examples/raft-kv-memstore-grpc/build.rs:7`  
```rust
// TODO: remove serde
```

### 2. Remove OptionSerde
**File:** `openraft/src/vote/raft_vote.rs:17`  
```rust
// TODO: OptionSerde can be removed after all types are made trait based.
```

### 3. Remove Membership State Limit
**File:** `openraft/src/core/raft_core.rs:529`  
```rust
// TODO: This limit can be removed if membership_state is replaced by a list of membership logs.
```

### 4. Remove Vote State Reader
**File:** `openraft/src/raft_state/vote_state_reader.rs:4`  
```rust
// TODO: remove it?
```

### 5. Remove Error Module Code
**File:** `openraft/src/error/mod.rs:209`  
```rust
// TODO: remove
```

### 6. Remove Unused Log Index Extension
**File:** `openraft/src/log_id/log_index_option_ext.rs:15`  
```rust
// TODO: unused, remove it
```

### 7. Remove Progress Module Code
**File:** `openraft/src/progress/mod.rs:24`  
```rust
// TODO: remove it
```

### 8. Remove Heartbeat Code
**File:** `tests/tests/append_entries/t61_heartbeat_reject_vote.rs:77`  
```rust
// TODO: this part can be removed when blank-log heartbeat is removed.
```

---

## Recommendations

### Immediate Actions (High Priority)
1. Address API changes and error handling issues to improve stability
2. Fix the unwrap() usage to prevent potential panics
3. Optimize large error types to reduce memory overhead

### Short-term Actions (Medium Priority)
1. Improve test coverage for critical components
2. Make configurable settings more flexible
3. Optimize performance bottlenecks (is_voter(), async methods)
4. Enhance documentation

### Long-term Actions (Low Priority)
1. Clean up deprecated and unused code
2. Refactor components for better maintainability
3. Consider architectural improvements

---

## Notes

- This analysis was performed on 2026-01-29
- TODOs were found by searching for "TODO" comments in `.rs`, `.md`, and `.toml` files
- Priority was assigned based on:
  - Impact on API stability and correctness
  - Performance implications
  - Code safety and error handling
  - Test coverage and reliability

For the latest TODO status, re-run the search:
```bash
grep -r "TODO" --include="*.rs" --include="*.md" --include="*.toml" -n .
```
