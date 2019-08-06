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
- once a node becomes leader and detects that its indnex is 0, it will commit a new config entry (instead of the normal blank entry created by new leaders).
- if a race condition takes place where two nodes persists an initial config and start an election, whichever node becomes leader will end up committing its entries to the cluster. When a node becomes leader and sees that its index is 0, it will generate a new initial commit with the current config, which will be replicated to the cluster.

##### ProposeConfigChange §6 | config change
This is for adding and removing nodes from a live cluster.

###### add
- [x] new node will come online in NonVoter state with pristine disk. Index & term set to 0.
- [x] all nodes will accept AppendEntries RPCs from any node claiming to be leader, as long as the leader's term is greater than current term (which will cause it to become the newly configured leader for that term) or is the currently configured leader of the current term.
- [x] stays non-voter until a config is replicated which marks it as a member instead of a non-voter.

###### remove
If a node ends up getting pushed back into non-voter state during initial cluster formation, but it is supposed to end up being a normal follower, the application will end up discovering the node, and will add it to the config via `ProposeConfigChange` and it will proceed as normal.

Applications which need to remove a node can safely do so after the node enters the non-voter state.

**todo:**
- [x] voting protocol is updated to include a field in the response RPC, `is_candidate_unknown`, which will be set to true if the candidate is not a member or non-voter.
- [x] if a candidate receives such a rejection and its index > 0, then it will go into non-voter state. If its index == 0, it will continue as a candidate as such is the case only during initial cluster formation.
- [x] need to update the `EntryConfigChange` model to include a field for `NonVoters` being added in the config & a new boolean field `is_joint` which indicates if the cluster is in a joint consensus phase or not.
- [x] update all locations where raft membership config is updated. Use a method to encapsulate logic. Will cause node to enter/exit non-voter state.
- [x] need to introduce the `is_non_voter` Raft state field. Non-voting nodes do not have election timeouts and are not considered for majority in the commit process and are not solicited for votes. Add this field to metrics enum as well.
- [x] if a leader is stepping down, it will ensure that its terminal config is committed to the cluster before transitioning to non-voter state.
- [x] if a new leader comes to power with a config in joint consensus, it must use its blank payload to ensure that the joint consensus has been committed.
- [x] replication handler on leader must monitor joint consensus as replication events arrive and committ a uniform consensus config when needed.
- [x] need to ensure that replication streams are brought down when nodes are removed.
- [x] when a new joint consensus config is presented, new_nodes being removed should immediately be removed and their replication streams should be brought downn.
- [x] when a new config is presented to the leader, or received by a follower from the leader, it will cause Raft to enter into the JointConsensus phase.
    - [x] new cluster members will always be added as `NonVoters`.
    - [x] ensure that all new nodes are brought up-to-speed before moving forward with reconfiguration. Replication stream must come up to line rate. Once all new nodes are line rate, the config will proceed, and the leader will generate the new `EntryConfigChange` and will commit it as the end of the joint consensus.
    - [x] (NOTE: this is implemented by the node going into non-voter state when it detects that it is no longer part of the cluster config): ensure that a leader node which is being removed from the cluster config properly transitions out of leader state once it commits the terminal entry for the config change.
    - [x] if a node is elected during a joint consensus phase, it must be cognizent of whether it will be phased out when the current joint consensus is committed.

**Notes:**
> For the configuration change mechanism to be safe, there must be no point during the transition where it is possible for two leaders to be elected for the same term. Unfortunately, any approach where servers switch directly from the old configuration to the new configuration is unsafe. It isn’t possible to atomically switch all of the servers at once, so the cluster can potentially split into two independent majorities during the transition.

> In Raft the cluster first switches to a transitional configuration we call joint consensus; once the joint consensus has been committed, the system then transitions to the new configuration. The joint consensus combines both the old and new configurations.

- Log entries are replicated to all servers in both configurations.
- Any server from either configuration may serve as leader.
- Agreement (for elections and entry commitment) requires majority from both the joint old + new configuration.
- When a node detects that it is no longer present in the cluster membership, it must transition to NonVoter state for application to shutdown as needed.

----

- [ ] check all location's where Raft.stop() or ReplicationStream.terminate() is used. Make sure cleanup is good. May need to use terminate on raft too.
- [ ] guard against sending snapshot pointers during replication.
- [ ] snapshot creation needs to be triggered based on configuration & distance from last snapshot.
- [ ] probably: make the Raft, RaftStorage, RaftNetwork & ReplicationStream types generic over a type `T` which will be the contents of the `EntryType::Normal`.
- [ ] maybe: may need to have `save_hard_state` finish before responding to RPCs in all conditions. Might be good to experiment with async/await here to help aid in the added complexity this would bring.
- [ ] maybe: update the append entries algorithm for followers so that recent entries are buffered up to a specific threshold so that the `apply_logs_to_statemachine` won't need to fetch from storage first.

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
