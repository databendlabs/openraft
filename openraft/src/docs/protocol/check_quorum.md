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
attempts therefore require periodic heartbeat to be enabled. Delayed ticks may
skip a send period and wait for the next one.

Log replication and snapshots are unchanged. Their responses can move `T`
forward, while a delayed replication RPC to one follower may renew that
follower's lease without forming a quorum.

The probe interval, `R`, must be at least
`leader_lease + election_timeout_max`. This is a nominal liveness floor, not a
Raft safety requirement: let the previous follower lease expire, then leave one
maximum election timeout for another election before allowing heartbeats. The
Leader lease currently equals `Config::election_timeout_max`. Add network and
scheduling margin and cover all voters' timing configurations. This local
bound is not an unconditional liveness guarantee: continued replication may
renew a follower lease after heartbeat suppression begins.

This policy does not modify Vote, public Leader identity, replication tasks, or
storage, and it adds no activity state. Quorum aggregation uses the effective
membership, excluding learners and covering joint membership. Leaving
`quorum_loss_probe_interval` unset preserves existing RPC behavior. The default
favors stable leadership during intermittent failures, but old-Leader
heartbeats can keep a bridge voter leased and prevent election in the topology of
[issue #2080](https://github.com/databendlabs/openraft/issues/2080). Enabling
inactivity gives the connected quorum an election opportunity.

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

During a suppression period, ReadIndex cannot send a heartbeat. During a send
period, it can trigger one normally. Its acknowledgement thresholds, waits, and
errors stay unchanged. Replication or snapshot responses may also complete a
pending read; otherwise its existing deadline expires.

[`ReadPolicy::LeaseRead`]: crate::ReadPolicy::LeaseRead
[`ReadPolicy::ReadIndex`]: crate::ReadPolicy::ReadIndex

## Difference from conventional CheckQuorum

Conventional CheckQuorum changes an isolated leader to the follower role after
an election timeout. OpenRaft instead separates the durable leadership grant
from the leader's current authority to accept new proposals:

| Behavior | Conventional CheckQuorum | OpenRaft default | OpenRaft with inactivity |
|----------|--------------------------|------------------|--------------------------|
| Lease expires | Become follower | Reject new proposals | Reject new proposals; start a heartbeat suppression period |
| Heartbeats | Stop | Continue | Alternate suppression and send periods |
| Replication and snapshots | Stop | Continue | Continue |
| Pending requests | Usually failed | Remain pending | Remain pending |
| Quorum communication recovers | Run a new election | Renew the lease | Renew the lease and resume heartbeats |

The OpenRaft behavior avoids an unnecessary election when the committed leader
can re-establish contact with a quorum, while still giving new client requests
an immediate routing signal.
