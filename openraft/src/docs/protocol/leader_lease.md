# Leader lease

Leader lease is a mechanism to prevent split-brain scenarios in which multiple leaders are elected in a cluster.

It is implemented by the leader sending heartbeats (append-entries and install-snapshot request
can be considered as heartbeat too) to the followers.
If the followers do not receive any heartbeats within a certain time period `lease`,
they will start an election to elect a new leader.


## The Lease for Leader and Follower

The `lease` stored on the leader is smaller than the `lease` stored on a follower.
The leader must not believe it is still valid while the follower has already started an election.


## Extend the lease

- A follower will extend the lease to `now + lease + timeout` when it receives a valid heartbeat from the leader, i.e., `leader.vote >= local.vote` .

- A leader will extend the lease to `t + lease` when a quorum grants the leader at time `t`.

  When a heartbeat RPC call succeeds,
  it means the target node acknowledged the leader's clock time when the RPC was sent.

  If a time point `t` is acknowledged by a quorum, the leader can be sure that no
  other leader existed during the period `[t, t + lease]`. The leader can then extend its
  local lease to `t + lease`.

The above `timeout` is the maximum time that can be taken by an operation that relies on the lease on the leader.


## Election-Related Timeout Configurations

The timeout configs are specifically designed to efficiently resolve election
conflicts.
Different timeout settings are used for committed and uncommitted votes.
This is because a committed vote indicates the presence of an established
**leader**, while an uncommitted vote merely represents a **candidate**.


For an uncommitted vote (the vote that is saved upon receiving a `VoteRequest`):

- An incoming `VoteRequest` will be allowed and handled at any time;
- The node **WILL NOT** elect itself until `election_timeout` has elapsed.

```text

             .- Election timeout
             |
--------+----+--------------> wall clock time
        |
        '- Vote.last_update_time
```

For a committed vote (the vote that is saved upon receiving `AppendEntriesRequest` or `InstallSnapshotRequest`):

- The node **WILL NOT** handle a `VoteRequest` before the `leader_lease` expires;
- The node **WILL NOT** elect itself until `leader_lease + election_timeout` has passed.

```text

             .--------- Accept VoteRequest
             |
             |       .- Next election time
             |       |
--------+----+-------+--------------> wall clock time
        |
        '- Vote.last_update_time

```
