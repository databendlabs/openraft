# Openraft Document

If you're starting to build an application with Openraft, check out
- [`getting_started`](crate::docs::getting_started).
- [`faq`](crate::docs::faq),

To maintain an Openraft cluster, e.g., add or remove nodes, refer to
- [`cluster_control`](crate::docs::cluster_control) :
  - [`cluster_formation`](`crate::docs::cluster_control::cluster_formation`) describes how to form a cluster;
  - [`dynamic membership`](`crate::docs::cluster_control::dynamic_membership`) describes how to add or remove nodes without downtime;
  - [`joint_consensus`](`crate::docs::cluster_control::joint_consensus`) describes detail of joint consensus implementation;
  - [`node lifecycle`](`crate::docs::cluster_control::node_lifecycle`) describes the transition of a node's state;

When upgrading an Openraft application, consult:
- [`upgrade_guide`](crate::docs::upgrade_guide) :
  - [`v0.6-to-v0.7`](`crate::docs::upgrade_guide::upgrade_06_07`);
  - [`v0.7-to-v0.8`](`crate::docs::upgrade_guide::upgrade_07_08`);
  - [`v0.8.3-to-v0.8.4`](`crate::docs::upgrade_guide::upgrade_083_084`);
  - [`v0.8-to-v0.9`](`crate::docs::upgrade_guide::upgrade_08_09`);

To learn about the data structures used in Openraft and the commit protocol, see
- [`feature_flags`](crate::docs::feature_flags);
- [`data`](crate::docs::data) :
  - [`Vote`](`crate::docs::data::vote`) is the core in a distributed system;
  - [`Log pointers`](`crate::docs::data::log_pointers`) shows how Openraft tracks entries in the log;
  - [`IOId`](`crate::docs::data::io_id`) explains the I/O operation identifier for both vote and log operations;
  - [`LogIOId`](`crate::docs::data::log_io_id`) explains the monotonic I/O operation identifier for log entries;
  - [`Log I/O Progress`](`crate::docs::data::log_io_progress`) explains how Openraft tracks log I/O operations through three stages;
  - [`Leader lease`](`crate::docs::data::leader_lease`) explains leader lease for a Leader and a Follower;
  - [`Extended membership`](`crate::docs::data::extended_membership`) explains how members are organized in Openraft;
  - [`Effective membership`](`crate::docs::data::effective_membership`) explains when membership config takes effect;
- [`components`](`crate::docs::components`) explains the components in Openraft;
  - [`RaftStateMachine`](`crate::docs::components::state_machine`) is the core API for managing the state machine and snapshot functionalities;
- [`protocol`](crate::docs::protocol) :
  - [`io_ordering`](`crate::docs::protocol::io_ordering`) explains why IO re-ordering is disallowed;
  - [`read`](`crate::docs::protocol::read`) explains how to do linearizable read;
  - [`replication`](`crate::docs::protocol::replication`);
    - [`leader_lease`](`crate::docs::protocol::replication::leader_lease`) outlines the leader validity criteria for Leaders and Followers;
    - [`log_replication`](`crate::docs::protocol::replication::log_replication`) provides an overview of the general replication protocol;
    - [`log_stream`](`crate::docs::protocol::replication::log_stream`) discusses the core aspects of streaming log replication;
    - [`snapshot_replication`](`crate::docs::protocol::replication::snapshot_replication`);

Contributors who want to understand the internals of Openraft can find relevant information in
- [`internal`](crate::docs::internal) :
  - [Architecture](`crate::docs::internal::architecture`) shows the overall architecture of Openraft;
  - [Threading](`crate::docs::internal::threading`) describes the threading model of Openraft;

Finally, the archived and discarded documents:
- [`obsolete`](crate::docs::obsolete) describes obsolete design documents and why they are discarded;
  - [`blank-log-heartbeeat`](`crate::docs::obsolete::heartbeat`);
  - [`fast-commit`](`crate::docs::obsolete::fast_commit`);
