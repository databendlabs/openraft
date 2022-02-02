# Example distributed KV store built upon openraft

It is an example of how to build a real-world kv store with openraft.
Includes:
- An in-memory `RaftStorage` implementation [store](./store).

- A server is based on [actix-web](https://docs.rs/actix-web/4.0.0-rc.2).  
  Includes:
  - raft-internal network APIs for replication and voting.
  - Admin APIs to add nodes, change-membership etc.
  - Application APIs to write a value by key or read a value by key.

- Client and `RaftNetwork`([network](./network)) are built upon [reqwest](https://docs.rs/reqwest).

## Run it

```shell
cd example-raft-kv
cargo build
./test-cluster.sh
```

## Cluster management

The raft itself does not store node addresses.
But in a real-world application, the implementation of `RaftNetwork` needs to know the addresses.

Thus, in this example application:

- The storage layer has to store nodes' information.
- The network layer keeps a reference to the store so that it is able to get the address of a target node to send RPC to.

To add a node to a cluster, it includes 3 steps:

- Write a `node` through raft protocol to the storage.
- Add the node as a `Learner` to let it start receiving replication data from the leader.
- Invoke `change-membership` to change the learner node to a member.