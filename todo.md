todo
====
### admin commands
##### InitWithConfig
This should be the only initialization command needed.

- pertains to cluster formation.
- works for single-node or multi-node cluster formation.
- recommennded pattern is for applications to have an `initial_cluster_formation_delay`-like config param. Application nodes should wait for that amount of time and then issue the `InitWithConfig` command.
- command should be called with all discovered nodes, including the calling node's ID. Command will be rejected if the node is not at index 0 & in the NonVoter state, as if either of those constraints are false, then the cluster has already formed and started.
- will set the given config as the active config, and will start an election.
- all nodes must do this at startup, as they will not be able to vote for other nodes until they appear in their config. The command will ensure that it will not go through if it is not safe to do so.
- once a node becomes leader and detects that its index is 0, it will commit a new config entry (instead of the normal blank entry created by new leaders).
- if a race condition takes place where two nodes persists an initial config and start an election, whichever node becomes leader will end up committing its entries to the cluster. When a node becomes leader and sees that its index is 0, it will generate a new initial commit with the current config, which will be replicated to the cluster.
- If a node ends up getting pushed back into non-voter state during initial cluster formation, but it is supposed to end up being a normal follower, the application will end up discovering the node, and will add it to the config via `ProposeConfigChange` and it will proceed as normal.

**todo:**
- [ ] update docs on admin commands.

----

- [ ] probably: make the Raft, RaftStorage, RaftNetwork & ReplicationStream types generic over a type `T` which will be the contents of the `EntryType::Normal`.
- [ ] check all location's where Raft.stop() or ReplicationStream.terminate() is used. Make sure cleanup is good. May need to use terminate on raft too.
- [ ] guard against sending snapshot pointers during replication.
- [ ] open ticket for: snapshot creation needs to be triggered based on configuration & distance from last snapshot.
- [ ] open ticket for: update the append entries algorithm for followers so that recent entries are buffered up to a specific threshold so that the `apply_logs_to_statemachine` won't need to fetch from storage first.

----

### observability
- [ ] instrument code with tokio trace: https://docs.rs/tokio-trace/0.1.0/tokio_trace/

----

### docs
- [ ] add a simple graphic to an `overview` section of the main README to visually demonstrate the layout of this system.
- [ ] put together a networking guide on how to get started with building a networking layer.
- [ ] put together a storage guide on how to implement an application's RaftStorage layer.
- [ ] add docs examples on how to instantiate and start the various actix actor components (Raft, RaftStorage, RaftNetwork &c).
- [ ] add docs on how metrics system works. Pumps out regular metrics configured interval. State changes to the node are always immediately published.
- [ ] add docs on the config change process & the fact that an alternative failsafe is used by this implementation for the third reconfiguration issue, removed node disruptions, described at the end of §6, and which is an improvement over the recommendation in the spec. This implemention does not accept RPCs from members which are not part of its configuration. The only time this will become an issue is:
    - the leader which is committing the terminal configuration which removes some set of nodes dies after it has only replicated to a minority of the cluster.
    - a member of that minority becomes leader and stops communicating with the old nodes.
    - if the new leader is not able to commit a new entry to the rest of the cluster before the old nodes timeout, it is possible that one of the old nodes will start an election and become the leader.
    - if this happens, that node will simply commit the terminal config change which removes it from the cluster and step down.
    - such conditions would be extremely rare, and are easily accounted for without any additional complexity in implementation.
- [ ] add docs to general application building section about client interactions. Per the Raft spec in §8, to ensure that client interactions are not applied > 1 due to a failure scenario and the client issuing a retry, applications must track client IDs and use serial numbers on each request. The RaftStorage implementaion may use this information in the client request log and reject the request from within the RaftStorage handler using an application specific error. The application's client may observe this error and treat it as an overall success. This is an application level responsibility, Raft simply provides the mechanism to be able to implement it.
