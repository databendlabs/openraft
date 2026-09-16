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

Set `Config::quorum_loss_probe_interval` to `Some(R)` to suppress ordinary
heartbeats when quorum is unavailable. `R` is the duration in milliseconds of
each alternating suppression and send period. `None` preserves the existing
runtime behavior.

Let `T` be the sending time of the last RPC acknowledged by a quorum. Before
the first quorum acknowledgement, OpenRaft uses the current Vote's
last-modified time. Heartbeats are allowed while `now - T < leader_lease`.
After that lease expires, the heartbeat gate uses:

```text
slot = (now - T - leader_lease) / R
```

Even-numbered slots suppress heartbeats; odd-numbered slots allow them. Thus
the first period after lease expiry is quiet. A new quorum acknowledgement
moves `T` forward and immediately restores normal heartbeats. A single-voter
Leader always leaves the gate open.

The gate applies to periodic, ReadIndex-triggered, and externally triggered
heartbeats. It does not schedule heartbeat attempts itself. Automatic recovery
attempts therefore require periodic heartbeat to be enabled.

Log replication and snapshots are not gated. Their responses can move `T`
forward.

`R` must be at least `leader_lease + election_timeout_max`, allowing a previous
follower lease to expire and one maximum election timeout to pass before
heartbeats resume.

This gives the connected quorum an election opportunity in the topology from
[issue #2080](https://github.com/databendlabs/openraft/issues/2080).

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
it contacts a quorum to confirm leadership. With quorum-loss inactivity
enabled, its heartbeat follows the same gate described above.

[`ReadPolicy::LeaseRead`]: crate::ReadPolicy::LeaseRead
[`ReadPolicy::ReadIndex`]: crate::ReadPolicy::ReadIndex

## Difference from conventional CheckQuorum

Conventional CheckQuorum changes an isolated leader to the follower role after
an election timeout. OpenRaft instead separates the durable leadership grant
from the leader's current authority to accept new proposals:

| Behavior | Conventional CheckQuorum | OpenRaft default |
|----------|--------------------------|------------------|
| Lease expires | Become follower | Reject new proposals |
| Heartbeats and replication | Stop | Continue |
| Pending requests | Usually failed | Remain pending |
| Quorum communication recovers | Run a new election | Renew the lease |

The default OpenRaft behavior avoids an unnecessary election when the committed
leader can re-establish contact with a quorum, while still giving new client
requests an immediate routing signal.
