# Openraft v0.9.25

A bug-fix-only release. Nine fixes, all backported from `release-0.10`, covering one
safety defect, one state-divergence defect, and a set of liveness problems that could
stall a leader, a follower, or a caller indefinitely.

No public API changes, no storage format changes. Upgrading is a version bump.

**Upgrading from 0.9.x is strongly recommended.** The commit-safety fix (#1802) and the
membership-divergence fix (#1808) can both corrupt cluster state in ways that do not heal
on their own.

## Safety

**Commit could be granted on a quorum that never existed** ([#1802], [7d8bf714])

`VecProgress` keeps every voter entry above `granted` in descending order, and the quorum
scan relies on that ordering: for each candidate value it tests whether the prefix
`vector[0..=i]` forms a quorum. The reordering step ran only when the updated voter had
previously been at or below `granted`; a voter already above it that advanced further kept
its old slot, leaving the region unsorted. The scan then counted an arbitrary prefix as
"the voters that reached this value" and could commit a log entry that fewer than a quorum
had actually accepted.

The regression test replays the five-voter sequence from the issue: before the fix it
granted index 6 with only two of five voters having reached it.

**Snapshot install could leave a node on a membership config that no longer exists**
([#1808], [7b66adf2])

A follower that had accepted a membership entry from an old leader, then installed a
snapshot whose last membership sits at a lower log index, kept its local effective
membership — while the same install set `purge_upto` to the snapshot's last log id and
deleted the entry backing that membership. The node was left running a config that neither
its log nor its snapshot could restore, permanently diverged from the rest of the cluster.

`MembershipState::update_committed()` now also considers the snapshot's last log index and
resets both committed and effective membership when the effective one falls inside the
purged range.

## Liveness

**Leader froze while rebuilding replication streams** ([#1810], [2447bae4])

`remove_all_replication()` awaited every removed replication task inline, inside the
RaftCore loop. A task parked in an `AppendEntries` call to an unresponsive follower only
returns after the RPC times out, so each rebuild stalled RaftCore for up to one RPC timeout
per removed target — and rebuilds happen on every membership change and every leadership
establishment. While parked, RaftCore serves neither writes, nor reads, nor metrics.

Removed tasks are now joined in the background. Progress from a detached task is discarded
by the existing `session_id` check, so it cannot leak into the new stream.

**Callers hung forever when the state machine worker died** ([211b91ba])

`client_write()`, `get_snapshot()` and `begin_receiving_snapshot()` resolved a closed
response channel by awaiting the RaftCore task handle. That is only correct when the core
actually stopped. The state machine worker owns the responders for the commands it serves,
so if that task dies on its own the responders drop while RaftCore keeps running — and the
caller awaits a handle that never resolves.

Both response paths now wait a bounded one second for the core to stop before falling back
to `Fatal::Stopped`. A genuine shutdown still reports the error that caused it; a dropped
responder returns promptly. The wait observes the metrics watch channel rather than joining
the task, so it is non-destructive and safe for any number of concurrent callers.

**Follower behind a fully purged log never received a snapshot** ([#1828], [81e304b5])

When the leader had purged its entire log (`purge_upto == last_log_id`) and a follower's
progress was reset, `next_send()` computed an empty range and returned `Inflight::None` on
every call. Nothing advanced, so the follower never probed, never got a snapshot, and could
not converge until a new entry happened to be proposed.

The empty range now sends a snapshot, guarded by `matching.next_index() < searching_end` so
that fully caught-up followers — which also reach `start == end` — are not shipped a
snapshot on every idle call.

**Replication task panicked on an empty `limited_get_log_entries()`** ([#1601], [546ed868])

The method is documented never to return empty for a non-empty range, and replication
relied on that by unwrapping `logs.first()`, guarded only by a `debug_assert!` — which is
compiled out in release builds. A store violating the contract panicked the replication
task in exactly the builds that matter.

An empty read is now handled as a heartbeat, with a 10 ms sleep before retrying (the range
does not advance, so an immediate retry would spin) and a warning naming the offending
range. RaftCore rebuilds the stream after a task dies, so the practical 0.9 impact was a
spurious panic and a replication stall rather than an outage.

**`trigger().elect()` on the current leader cost the group its leader** ([d94e232e])

Campaigning does not tear down the internal leader: `state.vote` moves to a new uncommitted
term while the node keeps heartbeating under the old one. Those heartbeats keep refreshing
the voters' leader leases, and an unexpired lease is exactly what makes them reject the vote
request the campaign just sent. The campaign could not win, and repeating the trigger only
repeated the loop — losing the leader and inflating the term for nothing.

The trigger is now a no-op when the node is already the leader.

## Metrics

**`current_leader()` returned `None` for a non-voter leader** ([#1693], [89629e33])

Openraft does not require a leader to be a voter — leadership follows from a committed vote.
`calc_server_state()` reports `ServerState::Leader` for a non-voter leader, but
`current_leader()` filtered its result through `effective().is_voter()`, so the same node
reported itself as `Leader` while `current_leader` was `None`. This is reachable whenever a
membership change removes the leader from the voter set; the node keeps operating and
committing until it steps down or a new leader is elected.

The check is removed, which also drops an `is_voter()` scan from a hot metrics path.

## Tooling

- **Publish script no longer aborts on already-published crates** ([c9520f1d]) — crates.io
  rejects API requests without a descriptive User-Agent (403), so every existence check read
  as "not published" and re-running after a partial publish always aborted. The check now
  sends a User-Agent, treats "already exists on crates.io index" as success, polls for
  indexing instead of sleeping a flat 30 s, and logs the resolved HTTP status.

## Behavior changes

These are behavior changes rather than API changes; no signature in the public surface moved.

- `RaftMetrics::current_leader` now reports a leader that is not a voter, where it previously
  reported `None`. Code that used `current_leader.is_some()` as an implicit "the leader is a
  voter" test needs an explicit check.
- `Raft::trigger().elect()` is a no-op on a node that is already the leader, instead of
  starting a campaign.

## Notes

All nine fixes are backports from `release-0.10`. Three are hand-ports rather than
cherry-picks, because the surrounding code was restructured upstream:

- [546ed868] — 0.10 fixes this in `stream_state.rs`, which does not exist on 0.9.
- [81e304b5] — 0.10 splits `next_send` into probing and pipeline regimes; 0.9 has no
  pipeline mode, so the probing condition is added to the existing shape.
- [d94e232e] — the Pre-Vote half of the upstream change is dropped; 0.9 has no Pre-Vote and
  therefore no non-disruptive election trigger to fall back on.

[#1601]: https://github.com/databendlabs/openraft/issues/1601
[#1693]: https://github.com/databendlabs/openraft/issues/1693
[#1802]: https://github.com/databendlabs/openraft/issues/1802
[#1808]: https://github.com/databendlabs/openraft/issues/1808
[#1810]: https://github.com/databendlabs/openraft/issues/1810
[#1828]: https://github.com/databendlabs/openraft/issues/1828

[7d8bf714]: https://github.com/databendlabs/openraft/commit/7d8bf71402949d25fdaf51d801446e53581c6673
[211b91ba]: https://github.com/databendlabs/openraft/commit/211b91baf7dd371f9522e87cffc12ab619000a75
[546ed868]: https://github.com/databendlabs/openraft/commit/546ed8683b3d569fc0855bb5950d479682626b2d
[89629e33]: https://github.com/databendlabs/openraft/commit/89629e332ab0dcc8b7af6fa30054d7e8d0e51a51
[d94e232e]: https://github.com/databendlabs/openraft/commit/d94e232e8b12265d0eb217eed779a1c8c034bdf6
[81e304b5]: https://github.com/databendlabs/openraft/commit/81e304b589ee6935696fea2d33c0073c829f0412
[c9520f1d]: https://github.com/databendlabs/openraft/commit/c9520f1dea3f5a7d2e169c4081748f9008938f9a
[2447bae4]: https://github.com/databendlabs/openraft/commit/2447bae4f941ec2574afff907aa70be2b7788498
[7b66adf2]: https://github.com/databendlabs/openraft/commit/7b66adf26cde8c0332918ca036ff805cff55f2df
