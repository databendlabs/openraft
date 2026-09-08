# CheckQuorum

CheckQuorum prevents a leader that can no longer reach a quorum from accepting
new proposals indefinitely. It improves client behavior during a network
partition; Raft safety does not depend on it because a leader without a quorum
cannot commit new log entries.

## Quorum lease

The leader records the sending time of every acknowledged `AppendEntries` RPC.
This includes heartbeats and normal replication. From these timestamps,
OpenRaft calculates the latest time acknowledged by the effective quorum. Joint
membership requires a quorum of every voter set.

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

## Heartbeats

Automatic heartbeat broadcast stops only after the quorum lease has been
expired continuously for one extra `leader_lease`:

```text
now >= last_quorum_acked + leader_lease + leader_lease
```

A heartbeat renews the followers' leader leases, and a follower whose lease is
fresh rejects vote requests. A leader that keeps broadcasting heartbeats after
long losing quorum support would indefinitely renew the leases of still
reachable followers, and a connected quorum that keeps hearing such an old
leader could never elect a new leader. Suppressing heartbeats only after a
full extra lease keeps the normal case intact: a transient lapse shorter than
one lease is healed by the next heartbeat round, without an election. A leader
with no quorum acknowledgement at all, such as a newly established leader,
never suppresses heartbeats.
See: [Raft does not Guarantee Liveness in the face of Network
Faults](https://decentralizedthoughts.github.io/2020-12-12-raft-liveness-full-omission/).

An explicitly requested heartbeat is not suppressed: `Trigger::heartbeat()` and
the heartbeat round of a ReadIndex read are client- or operator-driven quorum
checks, and a successful acknowledgement renews the lease and restores
heartbeat broadcast.

Replication of already accepted log entries is not gated: that traffic is
bounded, because no new entry can be proposed while the lease is expired, and
it provides the recovery channel once quorum communication is restored.

## Recovery

Lease expiry does not change the leader's committed vote or
[`ServerState`][]. The leader stops broadcasting automatic heartbeats once the
lease has been expired for one extra `leader_lease` and continues replicating
existing logs. A later quorum acknowledgement renews the lease, heartbeat
broadcast resumes, and the leader automatically resumes accepting proposals.

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
it contacts a quorum to confirm leadership and remains available as a recovery
path.

[`ReadPolicy::LeaseRead`]: crate::ReadPolicy::LeaseRead
[`ReadPolicy::ReadIndex`]: crate::ReadPolicy::ReadIndex

## Difference from conventional CheckQuorum

Conventional CheckQuorum changes an isolated leader to the follower role after
an election timeout. OpenRaft instead separates the durable leadership grant
from the leader's current authority to accept new proposals:

| Behavior | Conventional CheckQuorum | OpenRaft |
|----------|--------------------------|----------|
| Lease expires | Become follower | Reject new proposals; stop automatic heartbeats after one extra lease |
| Heartbeats and replication | Stop | Backlog replication continues; explicit heartbeat triggers still work |
| Pending requests | Usually failed | Remain pending |
| Quorum communication recovers | Run a new election | Renew the lease |

The OpenRaft behavior avoids an unnecessary election when the committed leader
can re-establish contact with a quorum, while still giving new client requests
an immediate routing signal.
