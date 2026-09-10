# CheckQuorum

CheckQuorum prevents a leader that can no longer reach a quorum from accepting
new proposals indefinitely. It improves client behavior during a network
partition; Raft safety does not depend on it because a leader without a quorum
cannot commit new log entries.

## Quorum lease

The leader records the sending time of every `AppendEntries` RPC for which it
receives a valid response. A full success, partial success, or conflict all
prove that the follower processed the request, so each supplies activity
evidence. A `HigherVote` response instead ends the current Leader session.
From these timestamps, OpenRaft calculates the latest time acknowledged by the
effective quorum. Joint membership requires a quorum of every voter set.

The leader may accept a new proposal only while:

```text
now < last_quorum_acked + leader_lease
```

If there is no quorum acknowledgement, or the acknowledgement is older than
`leader_lease`, the proposal is rejected with an empty
[`ForwardToLeader`][]. The empty result prevents a client from forwarding the
request back to the same leader. The lease duration is
`Config::election_timeout_max`.

[`ForwardToLeader`]: crate::errors::ForwardToLeader

## Optional leader retirement

By default, lease expiry only rejects new proposals. Set
[`Config::quorum_loss_step_down`][] to [`StepDownPolicy::After(ms)`][] to also
retire a Leader that remains unable to contact a voter quorum. `ms` is an
additional grace period after the quorum lease expires.

The transition is cancellable: when the deadline is reached, OpenRaft checks
the quorum again and does nothing if communication has recovered. A new Leader
session or changed voter quorum receives a complete leader-lease observation
window before the grace period can expire. While the policy is enabled, a
fresh quorum acknowledgement moves the deadline to
`last_quorum_acked + leader_lease + ms`. A response whose recorded sending time
is already outside the lease window still updates the historical clock, but it
does not postpone retirement.

[`StepDownPolicy::Never`][] preserves the default behavior. Because retirement
is driven by the tick loop, the setting has no effect when
[`Config::enable_tick`][] is `false`.

`After(ms)` does not create extra probe traffic. If heartbeats are disabled, an
otherwise reachable but idle multi-voter cluster may retire its Leader unless
writes, snapshots, or ReadIndex requests produce enough responses to keep a
quorum active.

This is a liveness trade-off. `Never` avoids leadership churn during unstable
connectivity but can leave some network topologies unable to elect a new
Leader. `After(ms)` eventually releases the old leadership authority but can
cause additional elections during network instability.

[`Config::enable_tick`]: crate::Config::enable_tick
[`Config::quorum_loss_step_down`]: crate::Config::quorum_loss_step_down
[`StepDownPolicy::After(ms)`]: crate::StepDownPolicy::After
[`StepDownPolicy::Never`]: crate::StepDownPolicy::Never

## Recovery

With the default [`StepDownPolicy::Never`] policy, lease expiry does not change
the leader's committed vote or [`ServerState`][]. The leader continues sending
heartbeats and replicating existing logs. A later quorum acknowledgement renews
the lease, and the leader automatically resumes accepting proposals. The same
recovery applies while an `After(ms)` grace period is still pending.

If another leader has already been elected, quorum intersection prevents the
old leader from renewing its lease. A voter in the new leader's quorum rejects
the old vote, allowing the old leader to observe the higher vote and follow the
normal leader-transition path.

[`ServerState`]: crate::ServerState

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
while the node is still Leader, it contacts a quorum to confirm leadership and
its responses can cancel a pending retirement. After local retirement the node
no longer sends that quorum probe as Leader and instead returns the normal
not-Leader response.

[`ReadPolicy::LeaseRead`]: crate::ReadPolicy::LeaseRead
[`ReadPolicy::ReadIndex`]: crate::ReadPolicy::ReadIndex

## Difference from conventional CheckQuorum

Conventional CheckQuorum changes an isolated leader to the follower role after
an election timeout. OpenRaft instead separates the durable leadership grant
from the leader's current authority to accept new proposals:

| Behavior | Conventional CheckQuorum | OpenRaft |
|----------|--------------------------|----------|
| Lease expires | Become follower | Reject new proposals |
| Heartbeats and replication | Stop | Continue |
| Pending requests | Usually failed | Remain pending |
| Quorum communication recovers | Run a new election | Renew the lease |

This table describes the default `Never` policy. It avoids an unnecessary
election when the committed leader can re-establish contact with a quorum,
while still giving new client requests an immediate routing signal. The
optional `After(ms)` policy instead retires the Leader if the lease and grace
period both expire without quorum recovery.
