# Leader Lease

Leader lease is a key concept in maintaining consistency and coordination among
nodes in distributed systems. It defines a time period during which nodes
believe that no other Leader will emerge in the cluster. Leader lease has
different meanings and roles for Follower and Leader nodes.

## Leader Lease for Follower Nodes

For Follower nodes, Leader lease means:

1. During the valid period of the Leader lease, Follower nodes will not initiate
   a new Leader election.
2. When a Follower node receives an AppendEntries request from the Leader, it
   refreshes its Leader lease.

It is important to note that receiving a RequestVote request does not trigger
the refresh of the Leader lease because Follower nodes do not consider a
RequestVote request to indicate that a Leader has been established.

## Leader Lease for Leader Nodes

For Leader nodes, Leader lease has the following implications:

1. During the valid period of the Leader lease, the Leader node is confident
   that there are no other Leaders in the cluster.
2. When the Leader node receives acknowledgments for the AppendEntries request
   from a quorum (majority) of Follower nodes, it refreshes its own Leader
   lease.

Note that the starting time for the Leader node to refresh its lease is the time
when the AppendEntries request is sent, not the time when the response is
received. This is because if the Leader uses the time when the response is
received as the starting time, due to network latency and clock differences,
there may be significant differences in the lease duration between the Leader
and Follower nodes. This could cause Follower nodes to prematurely consider the
Leader's lease to have expired, triggering unnecessary Leader elections.

It is also for this reason that a newly elected Leader has a lease of 0 before
it sends any AppendEntries requests. Additionally, the lease duration stored on
the Leader node is usually shorter than the lease duration stored on Follower
nodes.

## Leader Lease Example

The following diagram illustrates the different handling of Leader lease for
Leader and Follower nodes in the Raft consensus algorithm.

1. At time point t0, Node 1, as a Candidate, sends RequestVote requests to other
   nodes.

2. At time points t1 and t2, Node 2 and Node 3 reply with RequestVote responses,
   respectively, but they do not update their own Leader leases.

3. At time point t3, Node 1 receives enough votes to become the Leader. It
   starts sending AppendEntries requests to Follower nodes (Node 3 and Node 4).

4. At time point t4, after receiving the AppendEntries request, Node 4
   immediately updates its own Leader lease, with a lease duration of
   `config.lease`. Node 3 behaves similarly.

5. At time point t5, the Leader (Node 1) updates its own Leader lease only after
   receiving AppendEntries responses from a Quorum of all Follower nodes (3/4,
   including Node 1 itself). However, it is worth noting that:

   - Before time t5, the Leader lease on Node 1 is 0 because before t5, Node 1
     cannot ensure that there are no other nodes that can be elected as Leader
     in the cluster.

   - The starting time for the Leader's lease is calculated from the time point
     t3 when it sends the AppendEntries requests, not from the time point t5
     when it receives the responses. This is because at time t5, Node 1 can only
     guarantee that there are no other Leaders in the cluster during the time
     period `[t3, t3 + config.lease)`.

   - Similarly, it can also be inferred that the lease duration on the Leader is
     shorter than that on the Follower nodes.


```text
                                 [------------- leader lease -------------)
                                 *------------- config.lease -------------*
N4 -------------------------+----+--------------------------------------------------------->
                            ^    |              [------------- leader lease -------------)
                            |    '----------.   *------------- config.lease -------------*
N3 ---+------------+------+-|---------------|---+------------------------------------------>
      ^            |      ^ |               |   |
      |            |      | |               |   '--.
N2 -+-|------------|--+---|-|---------------|------|--------------------------------------->
    ^ |            |  |   | |               |      |
    | |RequestVote |  |   | |AppendEntries  |      |
    |/             v  v   |/                v      v
N1 -+--------------+--+---+-----------------+------+--------------------------------------->
    |                                              [- leader lease-)
    |                     .------------------------'
    |                     v
    |                     *------------- config.lease -------------*
    |                     |
    Candidate             Leader established

----+--------------+--+---+------+-----------------+--------------------------------------->
    t0             t1 t2  t3     t4                t5                                   time
```
