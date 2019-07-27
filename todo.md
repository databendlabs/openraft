todo
====
### algorithm optimizations
- [ ] Cache AppendEntries RPC entries on follower state so that pipeline for applying entries to SM can be more performant.
- [ ] may need to have `save_hard_state` finish before responding to RPCs in all conditions. Might be good to experiment with async/await here to help aid in the added complexity this would bring.
- [ ] maybe: update the append entries algorithm for followers so that recent entries are buffered up to a specific threshold so that the `apply_logs_to_statemachine` won't need to fetch from storage first.

### testing
- [ ] finish implement MemoryStroage for testing (and general demo usage).
- [ ] test snapshots.
    - cover case where cluster is making progress, and then a new node joins, but is not snapshotted because it is not too far behind.
    - cover case where new node node joins after cluster has make a lot of progress, and then new node should receive snapshot.

### snapshots
- [ ] get the system in place for periodic snapshot creation.
- [ ] guard against sending snapshot pointers during replication.

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

----

### admin commands
##### ProposeConfigChange §6 | config change
- need admin command for initiating a new reconfig. This begins a JoinConsensus throughout the cluster.
- need to introduce the `NonVoter` Raft state type. NonVoter nodes do not have election timeouts and are not considered for majority in the commit process and are not solicited for votes. Add this variant to metrics enum as well.
- need to update the `EntryConfigChange` model to include a field for `NonVoters` being added in the config & a new boolean field `is_joint` which indicates if the cluster is in a join consensus phase or not.
- when a new config is presented to the leader, or received by a follower from the leader, it will cause Raft to enter into the JointConsensus phase.
    - new cluster members will always be added as followers, this implementation will take care of making sure that they are brought up-to-speed before moving forward with reconfiguration. Replication stream must come up to line rate. Once all new nodes are line rate, the config will proceed, and the leader will generate the new `EntryConfigChange` and will commit it as the end of the joint consensus.
    - add an optional leader state field which indicates that the leader needs to step down after a specific offset has been committed. This is for when a node which is being phased out of cluster membership is the leader and is committing its terminal config change to end the joint consensus.
    - if a node is elected during a joint consensus phase, it must be cognizent of whether it will be phased out when the current joint consensus is committed.

**Notes:**
> For the configuration change mechanism to be safe,there must be no point during the transition where itis possible for two leaders to be elected for the sameterm. Unfortunately, any approach where servers switchdirectly from the old configuration to the new configuration is unsafe. It isn’t possible to atomically switch all ofthe servers at once, so the cluster can potentially split intotwo independent majorities during the transition.

> In Raft the clusterfirst switches to a transitional configuration we calljointconsensus; once the joint consensus has been committed,the system then transitions to the new configuration. The joint consensus combines both the old and new configurations.

- Log entries are replicated to all servers in both configurations.
- Any server from either configuration may serve asleader.
- Agreement (for elections and entry commitment) requires separate majorities from both the old and new configurations.
