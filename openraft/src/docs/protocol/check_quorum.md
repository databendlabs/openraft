# CheckQuorum

CheckQuorum prevents a leader that can no longer reach a quorum from accepting
new proposals indefinitely. It improves client behavior during a network
partition; Raft safety does not depend on it because a leader without a quorum
cannot commit new log entries.

## Quorum lease

The leader records the sending time of every acknowledged `AppendEntries` RPC.
This includes heartbeats, full and partial replication success, and conflict
responses; successful snapshot responses also advance this clock. From these
timestamps, OpenRaft calculates the latest time acknowledged by the effective
quorum. Joint membership requires a quorum of every voter set.

The leader may accept a new proposal only while:

```text
now < last_quorum_acked + leader_lease
```

If there is no quorum acknowledgement, or the acknowledgement is older than
`leader_lease`, the proposal is rejected with an empty
[`ForwardToLeader`][]. The empty result prevents a client from forwarding the
request back to the same leader. The quorum activity evidence window, denoted
`W`, is this leader lease duration: `W = leader_lease = Config::election_timeout_max`
milliseconds. It is measured from the acknowledged RPC's sending time, not
when the response arrives.

[`ForwardToLeader`]: crate::errors::ForwardToLeader

## Recovery

By default, lease expiry does not change the leader's committed vote or
[`ServerState`][]. The leader continues sending heartbeats and replicating
existing logs. A later quorum acknowledgement renews the lease, and the leader
automatically resumes accepting proposals.

If another leader has already been elected, quorum intersection prevents the
old leader from renewing its lease. A voter in the new leader's quorum rejects
the old vote, allowing the old leader to observe the higher vote and follow the
normal leader-transition path.

[`ServerState`]: crate::ServerState

## Optional quorum-loss inactivity

Set `Config::quorum_loss_probe_interval` to `Some(R)` to opt into pausing
ordinary Leader traffic when quorum is unavailable. `R` is the recovery-probe
interval in milliseconds. `None` preserves the existing runtime behavior.

The current Leader session has two internal activity states:

- `Active`: normal traffic continues. A new Leader observes one full
  `leader_lease` window before its first quorum-loss check. Later checks are
  scheduled at the exact expiry of the latest quorum evidence.
- `Inactive`: ordinary heartbeats, AppendEntries, and snapshot attempts pause.
  Requests already admitted may finish and their responses still count. Every
  probe interval, one empty AppendEntries attempt goes to each effective remote
  voter. Fresh evidence satisfying the membership quorum immediately resumes
  `Active`; a HigherVote follows the usual Leader transition.

When an Active check becomes due, the Leader enters `Inactive` immediately if
the current evidence no longer forms a quorum. There is no additional grace
period. Operators who prefer stable Leader traffic during intermittent network
loss can leave this feature disabled.

Inactivity does not modify Vote, public Leader identity, or storage. It retains
replication tasks and inflight work rather than cancelling them. A restart or
new Leader session starts active, and quorum aggregation still uses the
effective membership, excluding learners and covering joint membership.

Changing the effective voter set rebuilds quorum tracking but does not create
a special observation period or change the current activity deadline. New
voters initially have no evidence. A due Active check may therefore enter
`Inactive`; a probe round that obtains a fresh quorum from the new effective
voters restores normal traffic.

The probe interval, `R`, must be at least
`leader_lease + election_timeout_max`. This is a nominal liveness floor, not a
Raft safety requirement: let the previous follower lease expire, then leave one
maximum election timeout for another election to run before probing again. The
Leader lease currently equals `Config::election_timeout_max`. Add network and
scheduling margin and cover all voters' timing configurations. This local
bound is not an unconditional liveness guarantee: a delayed admitted RPC may
renew a follower lease after the pause.

The first probe interval starts when Core closes ordinary traffic admission.
Later intervals start when Core submits a probe round, not when Engine queues
it or its responses arrive. Delayed ticks do not cause catch-up rounds.

Tick drives activity checks and probes, independently of `enable_heartbeat`.
Disabling tick preserves state, deadlines, and the traffic permit; already
queued ticks and commands are not recalled. Qualifying responses may still
restore quorum immediately, and HigherVote processing continues. A membership
update may rebuild the evidence structure, but remains activity-state and
deadline neutral. The next tick after re-enabling evaluates the original
deadlines without restarting them.

Leaving `quorum_loss_probe_interval` unset creates no activity controller,
permit channel, or extra quorum-loss checks. Existing evidence maintenance and
RPC behavior are unchanged. This default favors stable leadership during
intermittent failures, but old-Leader traffic can keep a bridge voter leased
and prevent election in the topology of
[issue #2080](https://github.com/databendlabs/openraft/issues/2080). Enabling
inactivity gives the connected quorum an election opportunity at the cost of
possible pause/resume churn during unstable networking.

## Pending requests

Lease expiry affects only new proposals. Requests accepted while the lease was
valid remain pending because their log entries may still commit after quorum
communication recovers. Applications may apply their own request timeout; such
a timeout means that the result is unknown, not that the log entry was
discarded.

Rejecting new proposals bounds the amount of uncommitted work accumulated
during the partition without invalidating work already accepted.

## Reads

[`ReadPolicy::LeaseRead`][] uses the same quorum lease and returns an error when
the lease has expired. [`ReadPolicy::ReadIndex`][] does not rely on the lease:
it obtains fresh quorum evidence to confirm leadership. Normally this triggers
a heartbeat, including when periodic heartbeat is disabled.

LeaseRead only checks the existing lease; it does not contact followers or
advance `clock_progress`. With periodic heartbeat disabled, LeaseRead requests
alone cannot keep an idle multi-voter Leader's quorum lease fresh. ReadIndex
requests can obtain fresh heartbeat responses to renew it; replication and
snapshot responses also renew the lease.

While inactive, ReadIndex cannot send an ordinary heartbeat or request an extra
probe. Its acknowledgement thresholds, waits, and errors stay unchanged.
Responses from admitted RPCs or scheduled probes may complete a pending read;
otherwise its existing deadline expires. A read using the default
`leader_lease` wait can time out before the next probe because the probe
interval is longer.

[`ReadPolicy::LeaseRead`]: crate::ReadPolicy::LeaseRead
[`ReadPolicy::ReadIndex`]: crate::ReadPolicy::ReadIndex

## Difference from conventional CheckQuorum

Conventional CheckQuorum changes an isolated leader to the follower role after
an election timeout. OpenRaft instead separates the durable leadership grant
from the leader's current authority to accept new proposals:

| Behavior | Conventional CheckQuorum | OpenRaft default | OpenRaft with inactivity |
|----------|--------------------------|------------------|--------------------------|
| Lease expires | Become follower | Reject new proposals | Reject new proposals; enter Inactive at a due failed check |
| Heartbeats and replication | Stop | Continue | Pause at a due failed check; scheduled probes only |
| Pending requests | Usually failed | Remain pending | Remain pending |
| Quorum communication recovers | Run a new election | Renew the lease | Renew the lease and resume traffic |

The OpenRaft behavior avoids an unnecessary election when the committed leader
can re-establish contact with a quorum, while still giving new client requests
an immediate routing signal.
