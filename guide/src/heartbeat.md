# Heartbeat in openraft

## Heartbeat in standard raft

Heartbeat in standard raft is the way for a leader to assert it is still alive:
- A leader send heartbeat at a regular interval.
- A follower that receives a heartbeat believes there is an active leader thus it rejects election request(`send_vote`) from another node unreachable to the leader, for a short period.

## Openraft heartbeat is a blank log

Such a heartbeat mechanism depends on clock time.
But raft as a distributed consensus already has its own **pseudo time** defined very well.
Raft, or other consensus protocol has its own **pseudo time** defined internally:
- In [paxos](https://en.wikipedia.org/wiki/Paxos_(computer_science)) it is `round_number`(AKA ballot number in some paper).
- In the standard raft it is `(term, voted_for, last_log_index)`(because in standard raft there is only one leader in every term, `voted_for` can be removed: `(term, last_log_index)`).

The **pseudo time** in openraft is a tuple `(vote, last_log_id)`, compared in dictionary order(`vote` is equivalent concept as round number in Paxos).

### Why it works

To refuse the election by a node that does not receive recent messages from the current leader,
just let the active leader send a **blank log** to increase the **pseudo time** on a quorum.

Because the leader must have the greatest **pseudo time**,
thus by comparing the **pseudo time**, a follower automatically refuse election request from a node unreachable to the leader.

And comparing the **pseudo time** is already done by `handle_vote_request()`,
there is no need to add another timer for the active leader. 

Thus making heartbeat request a blank log is the simplest way.

## Why blank log heartbeat?

- Simple, get rid of a timer.

  Without heartbeat log(the way standard raft does), when handling a vote
  request, except `vote` itself, it has to examine two values to determine if
  the vote request is valid:
    - Whether the last heartbeat has expired by clock time.
    - Whether the `(last_term, last_log_index)` in the request is greater or equal to the local value. This is the pseudo time Raft defines.

  With heartbeat log(the way openraft does), when handling a vote request, it only needs to examine one value: the raft time: `(last_term, last_log_index)`. This makes the logic simpler and the test easier to write.

- Easy to prove, and reduce code complexity.


## Concerns

- **More raft logs are generated**.
  This requires to *persist* the blank entry in the log (or at least the incremented index).
  E.g., doing that every 50ms for 100 consensus domains on one machine will require 2000 IOPS alone for that.

  **Why it is not a problem**:

  1. Assume that most consensus domains are busy, and as a domain is busy, it is possible to merge multiple `append-entry` calls into one call to the storage layer.
     Thus if a domain swallows `10` business log entries per `50 ms`, it's likely to merge these 10 entries into one or a few IO calls.
     The IO amplification should be smaller as IOPS gets more.

     Merging entries into one IO call is naturally done on followers(because the leader sends entries in a batch).
     On the leader, it's not done yet(2022 Sep 13). It can be done when the Engine oriented refactoring is ready: (.

  2. If a consensus domain swallows `1` business log entry per `50 ms`. It does not need another heartbeat. A normal append-entry can be considered a heartbeat.
