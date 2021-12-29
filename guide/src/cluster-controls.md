# Cluster Controls

A raft cluster may be control in various ways using the API methods of the `Raft` type.
This allows the application to influence the raft behavior.

There are several concepts related to cluster control:

- Voter: a raft node that is responsible to vote, elect itself for leadership(Candidate),
    become Leader or Follower

- Candidate: a node tries to elect itself as the Leader.

- Leader: the only node in a cluster that deals with application request.

- Follower: a node that believes there is a legal leader and just receives
    replicated logs.

- Learner: a node that is not allow to vote but only receives logs.


Voter state transition:

```
                         vote granted by a quorum
          .--> Candidate ----------------------> Leader
heartbeat |     |                                  |
timeout   |     | seen a higher Leader             | seen a higher Leader
          |     v                                  |
          '----Follower <--------------------------'
```


