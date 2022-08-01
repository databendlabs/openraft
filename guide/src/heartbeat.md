# Heartbeat in openraft

## Heartbeat in standard raft

Heartbeat in standard raft is the way for a leader to assert it is still alive:
- A leader send heartbeat at a regular interval.
- A follower that receives a heartbeat believes there is an active leader thus it rejects election request(`send_vote`) from another node unreachable to the leader, for a short period.

## Openraft heartbeat is a blank log

Such a heartbeat mechanism depends on clock time.
But raft as a distributed consensus already has its own **pseudo time** defined very well.
The **pseudo time** in openraft is a tuple `(vote, last_log_id)`, compared in dictionary order.

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
- Easy to prove, and reduce code complexity.

## Other Concerns

- More raft logs are generated.





