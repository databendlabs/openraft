todo
====
- [ ] probably: make the Raft, RaftStorage, RaftNetwork & ReplicationStream types generic over a type `T` which will be the contents of the `EntryType::Normal`. Looks like RaftNetwork no longer needs generic constraints. Check.
- [ ] check all location's where Raft.stop() or ReplicationStream.terminate() is used. Make sure cleanup is good. May need to use terminate on raft too.
- [ ] guard against sending snapshot pointers during replication.

----

- [ ] open ticket for: snapshot creation needs to be triggered based on configuration & distance from last snapshot.
- [ ] open ticket for: update the append entries algorithm for followers so that recent entries are buffered up to a specific threshold so that the `apply_logs_to_statemachine` won't need to fetch from storage first.
- [ ] open ticket for: instrument code with tokio trace: https://docs.rs/tokio-trace/0.1.0/tokio_trace/

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
