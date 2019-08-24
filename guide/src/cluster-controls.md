Cluster Controls
================
Raft nodes may be controlled in various ways outside of the normal flow of the Raft protocol using the `admin` message types. This allows the parent application — within which the Raft node is running — to influence the Raft node's behavior based on application level needs.

### concepts
In the world of Raft consensus, there are a few aspects of a Raft node's lifecycle which are not directly dictated in the Raft spec. Cluster formation and the preliminary details of what would lead to dynamic cluster membership changes are a few examples of concepts not directly detailed in the spec. This implementation of Raft offers as much flexibility as possible to deal with such details in a way which is safe according to the Raft specification, but also in a way which preserves flexibility for the many different types of applications which may be implemented using Raft.
