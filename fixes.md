## Fixed bugs by openraft since async-raft-v0.6.1

### fix: too many(50) inconsistent log should not live lock append-entries
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
R2 ... 3,99 3,100
```

Before this fix, brings up the cluster, R2 becomes leader and will never sync any log to R0.

The fix is also quite simple:

- Search backward instead of searching forward, to find the last log
  entry that matches `prev_log_id.term`, and responds this log id to the
  leader to let it send next `append_entries` RPC since this log id.

- If no such matching term is found, use the first log id it sees, e.g.,
  the entry at index `prev_log_id.index - 50` for next `append_entries`.


### fix: RaftCore.entries_cache is inconsistent with storage. removed it.
- When leader changes, `entries_cache` is cleared.
  Thus there may be cached entries wont be applied to state machine.

- When applying finished, the applied entries are not removed from the
  cache.
  Thus there could be entries being applied more than once.


### fix: snapshot replication does not need to send a last 0 size chunk

### fix: install snapshot req with offset GE 0 should not start a new session.
A install-snapshot always ends with a req with data len to be 0 and
offset GE 0.
If such a req is re-sent, e.g., when timeout, the receiver will try to
install a snapshot with empty data, if it just finished the previous
install snapshot req(`snapshot_state` is None) and do not reject a
install snapshot req with offset GE 0.
Which results in a `fatal storage error`, since the storage tries to
decode an empty snapshot data.

- feature: add config `install_snapshot_timeout`.


### fix: leader should not commit when there is no replication to voters.
When there is no replication to voters but there are replications to
learners, the leader did not check learners for a quorum but just
commits a log at once.

This cause the membership change log from a single node always commits.
E.g. start node 0, and learner 1, 2; then `change_membership({0, 1, 2})`,
It just commits the joint-log at once.
But according to raft paper, it should await a quorum of {0} and a
quorum of {0, 1, 2}.


### fix: after 2 log compaction, membership should be able to be extract from prev compaction log

### fix: when finalize_snapshot_installation, memstore should not load membership from its old log that are going to be overridden by snapshot.

### fix: leader should re-create and send snapshot when `threshold/2 < last_log_index - snapshot < threshold`
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


### fix: client_read has using wrong quorum=majority-1

### fix: doc-include can only be used in nightly build
- Simplify CI test: test all in one action.

- Disable clippy: it suggests inappropriate assert_eq to assert
  convertion which is used in a macro.

- Add makefile

- Build only with nightly rust. Add rust-toolchain to specify toolchain
  version.


### fix: when handle_update_match_index(), learner should also be considered, because when member change a learner is also count as a quorum member

### fix: when calc quorum, the learner should be count
Counting only the follower(nodes) as quorum for new config(c1) results
in unexpected log commit.
E.g.: change from 012 to 234, when 3 and 4 are unreachable, the first
log of joint should not be committed.


### fix: a NonVoter should stay as NonVoter instead of Follower after restart

### fix: discarded log in replication_buffer should be finally sent.
Internally when replication goes to LaggingState(a non-leader lacks a lot logs), the
ReplicationCore purges `outbound_buffer` and `replication_buffer` and then sends all
**committed** logs found in storage.

Thus if there are uncommitted logs in `replication_buffer`, these log will never have chance to
be replicated, even when replication goes back to LineRateState.
Since LineRateState only replicates logs from `ReplicationCore.outbound_buffer` and
`ReplicationCore.replication_buffer`.

This test ensures that when replication goes to LineRateState, it tries to re-send all logs
found in storage(including those that are removed from the two buffers.


### fix: a conflict is expected even when appending empty enties

### fix: last_applied should be updated only when logs actually applied.

