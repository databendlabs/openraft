# Openraft Document

If you're starting to build an application with Openraft, check out
- [`getting_started`].
- [`faq`],

To maintain an Openraft cluster, e.g., add or remove nodes, refer to
- [`cluster_control`] :
  - [`cluster_formation`](`cluster_control::cluster_formation`) describes how to form a cluster;
  - [`dynamic membership`](`cluster_control::dynamic_membership`) describes how to add or remove nodes without downtime;
  - [`node lifecycle`](`cluster_control::node_lifecycle`) describes the transition of a node's state;

When upgrading an Openraft application, consult:
- [`upgrade_guide`] :
  - [`v0.6-to-v0.7`](`upgrade_guide::upgrade_06_07`);
  - [`v0.7-to-v0.8`](`upgrade_guide::upgrade_07_08`);

To learn about the data structures used in Openraft and the commit protocol, see
- [`feature_flags`] ;
- [`data`] :
  - [`Vote`](`data::vote`) is the core in a distributed system;
  - [`Log pointers`](`data::log_pointers`) shows how Openraft tracks entries in the log;
  - [`Extended membership`](`data::extended_membership`) explains how members are organized in Openraft;
  - [`Effective membership`](`data::effective_membership`) explains when membership config takes effect;
- [`protocol`] :
  - [`fast-commit`](`protocol::fast_commit`);

Contributors who want to understand the internals of Openraft can find relevant information in
- [`internal`] :
  - [Architecture](`crate::docs::internal::architecture`) shows the overall architecture of Openraft;
  - [Threading](`crate::docs::internal::threading`) describes the threading model of Openraft;

Finally, the archived and discarded documents:
- [`obsolete`] describes obsolete design documents and why they are discarded;
