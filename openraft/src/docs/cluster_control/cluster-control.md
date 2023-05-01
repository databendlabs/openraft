# Managing Clusters: Adding/Removing Nodes

The `Raft` type offers various API methods to control a raft cluster.
Several concepts are associated with cluster management:

- `Voter`: A raft node responsible for voting, electing itself as a `Candidate`, and becoming a `Leader` or `Follower`.

- `Candidate`: A node attempting to elect itself as the Leader.

- `Leader`: The sole node in a cluster that handles application requests.

- `Follower`: A node that acknowledges the presence of a valid leader and only receives replicated logs.

- `Learner`: A node that cannot vote but only receives logs.


`Voter` state transition:

```text
                         vote granted by a quorum
          .--> Candidate ----------------------> Leader
heartbeat |     |                                  |
timeout   |     | seen a higher Leader             | seen a higher Leader
          |     v                                  |
          '----Follower <--------------------------'
```
