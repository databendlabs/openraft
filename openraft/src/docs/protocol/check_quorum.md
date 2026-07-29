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

## Recovery

Lease expiry does not change the leader's committed vote or
[`ServerState`][]. The leader continues sending heartbeats and replicating
existing logs. A later quorum acknowledgement renews the lease, and the leader
automatically resumes accepting proposals.

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
| Lease expires | Become follower | Reject new proposals |
| Heartbeats and replication | Stop | Continue |
| Pending requests | Usually failed | Remain pending |
| Quorum communication recovers | Run a new election | Renew the lease |

The OpenRaft behavior avoids an unnecessary election when the committed leader
can re-establish contact with a quorum, while still giving new client requests
an immediate routing signal.
