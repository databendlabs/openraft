# Cluster Control

Openraft provides APIs to manage cluster membership, handle node lifecycle, and coordinate configuration changes safely.

## Node Roles

**Voter**: Participates in elections and voting. Can become candidate, leader, or follower.

**Leader**: Single node handling all client requests and coordinating log replication.

**Follower**: Accepts logs from leader and participates in voting during elections.

**Learner**: Receives replicated logs but doesn't vote. Useful for adding capacity or testing before promoting to voter.

**Candidate**: Temporary state when a voter requests votes to become leader.

## State Transitions

```text
                         vote granted by a quorum
          .--> Candidate ----------------------> Leader
heartbeat |     |                                  |
timeout   |     | seen a higher Leader             | seen a higher Leader
          |     v                                  |
          '----Follower <--------------------------'
```

## Cluster Operations

### Initialization

See: [`cluster_formation`]

Bootstrap a new cluster by initializing the first node with an initial membership configuration.

### Membership Changes

See: [`dynamic_membership`], [`joint_consensus`]

Add or remove nodes using safe, two-phase joint consensus protocol that maintains availability during configuration changes.

### Node Management

See: [`node_lifecycle`]

Handle node addition (learner → voter promotion), removal, and failure scenarios.

### Monitoring and Maintenance

See: [`monitoring_maintenance`]

Monitor cluster health using metrics and perform automated maintenance operations safely.

## Key APIs

**Initialize cluster**: `Raft::initialize()` - Bootstrap first node

**Add learner**: `Raft::add_learner()` - Add non-voting node for replication

**Change membership**: `Raft::change_membership()` - Modify cluster voter set

**Monitor state**: `Raft::metrics()` - Track node roles and cluster health

[`cluster_formation`]: `crate::docs::cluster_control::cluster_formation`
[`dynamic_membership`]: `crate::docs::cluster_control::dynamic_membership`
[`joint_consensus`]: `crate::docs::cluster_control::joint_consensus`
[`node_lifecycle`]: `crate::docs::cluster_control::node_lifecycle`
[`monitoring_maintenance`]: `crate::docs::cluster_control::monitoring_maintenance`
