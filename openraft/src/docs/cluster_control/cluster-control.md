# Cluster Control

Openraft provides APIs to manage cluster membership, handle node lifecycle, and coordinate configuration changes safely.

## Node Roles

**Voter**: Contributes to a read or write quorum.

**Campaigner**: May start an election.

**Leader**: Single node handling all client requests and coordinating log replication.

**Follower**: Accepts logs from the leader and has a campaign, read voter, or write voter role.

**Learner**: Receives replicated logs with no campaign, read voter, or write voter role. Useful for adding capacity or testing before promoting to voter.

**Candidate**: Temporary state while a campaigner requests votes to become leader.

The current `Membership` gives every voter all three roles.

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

Add or remove nodes safely, in one log entry when every old quorum intersects every new quorum, or otherwise through the two-phase joint consensus protocol, which maintains availability during configuration changes.

### Separate Read and Write Quorums

See: [`provided_memberships`], [`election_quorum`]

A membership sets the read quorums that elect a leader and the write quorums that commit an entry, and every read quorum shares a voter with every write quorum. The default `Membership` uses a majority for both. The `openraft-membership` crate provides other rules, selected with `RaftTypeConfig::Membership` and `RaftTypeConfig::MembershipEdit`:

- `ThresholdMembership` chooses how many copies a commit needs and derives the minimum intersecting election quorum; see [`threshold_membership`].
- `WeightedMembership` gives voters, or the sites that hold them, different weights.
- `HierarchicalMembership` needs enough members in enough groups, such as zones, so that the cluster can keep running after it loses a whole group.
- `GridMembership` lets a full row elect and a voter of each row commit, so in a square grid both wait for fewer voters than a majority.
- `ExplicitMembership` lists the quorums, for a policy that no rule above states.

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
[`threshold_membership`]: https://docs.rs/openraft-membership/latest/openraft_membership/docs/threshold/index.html
[`provided_memberships`]: https://docs.rs/openraft-membership/latest/openraft_membership/docs/index.html
[`election_quorum`]: `crate::docs::protocol::election_quorum`
[`node_lifecycle`]: `crate::docs::cluster_control::node_lifecycle`
[`monitoring_maintenance`]: `crate::docs::cluster_control::monitoring_maintenance`
