# Example distributed key-value store built upon openraft.

It is an example of how to build a real-world key-value store with `openraft`.
Includes:
- An in-memory `RaftStorage` implementation [store](./src/store/store.rs).

- A server is based on [actix-web](https://docs.rs/actix-web/4.0.0-rc.2).  
  Includes:
  - raft-internal network APIs for replication and voting.
  - Admin APIs to add nodes, change-membership etc.
  - Application APIs to write a value by key or read a value by key.

- Client and `RaftNetwork`([rpc](./src/network/raft_network_impl)) are built upon [reqwest](https://docs.rs/reqwest).

  [ExampleClient](./src/client.rs) is a minimal raft client in rust to talk to a raft cluster.
  - It includes application API `write()` and `read()`, and administrative API `init()`, `add_learner()`, `change_membership()`, `metrics()`.
  - This client tracks the last known leader id, a write operation(such as `write()` or `change_membership()`) will be redirected to the leader on client side.

## Run it

There is a example in bash script and an example in rust:

- [test-cluster.sh](./test-cluster.sh) shows a simulation of 3 nodes running and sharing data,
  It only uses `curl` and shows the communication between a client and the cluster in plain HTTP messages.
  You can run the cluster demo with:

  ```shell
  ./test-cluster.sh
  ```

- [test_cluster.rs](./tests/cluster/test_cluster.rs) does almost the same as `test-cluster.sh` but in rust
  with the `ExampleClient`.

  Run it with `cargo test`.


if you want to compile the application, run:

```shell
cargo build
```

(If you append `--release` to make it compile in production, but we don't recommend to use
this project in production yet.)

## What the test script does

To run it, get the binary `raft-key-value` inside `target/debug` and run:

```shell
./raft-key-value --id 1 --http-addr 127.0.0.1:21001
```

It will start a node.

To start the following nodes:

```shell
./raft-key-value --id 2 --http-addr 127.0.0.1:21002
```

You can continue replicating the nodes by changing the `id` and `http-addr`.

After that, call the first node created:

```
POST - 127.0.0.1:21001/init
```

It will define the first node created as the leader.

Then you need to inform to the leader that these nodes are learners:

```
POST - 127.0.0.1:21001/add-learner '[2, "127.0.0.1:21002"]'
POST - 127.0.0.1:21001/add-learner '[3, "127.0.0.1:21003"]'
```

Now you need to tell the leader to add all learners as members of the cluster:

```
POST - 127.0.0.1:21001/change-membership  "[1, 2, 3]"
```

Write some data in any of the nodes:

```
POST - 127.0.0.1:21001/write  "{"Set":{"key":"foo","value":"bar"}}"
```

Read the data from any node:

```
POST - 127.0.0.1:21002/read  "foo"
```

You should be able to read that on the another instance even if you did not sync any data!


## How it's structured.

The application is separated in 4 modules:

 - `bin`: You can find the `main()` function in [main](./src/bin/main.rs) the file where the setup for the server happens.
 - `network`: You can find the [api](./src/network/api.rs) that implements the endpoints used by the public API and [rpc](./src/network/raft_network_impl) where all the raft communication from the node happens. [management](./src/network/management.rs) is where all the administration endpoints are present, those are used to add orremove nodes, promote and more. [raft](./src/network/raft.rs) is where all the communication are received from other nodes.
 - `store`: You can find the file [store](./src/store/mod.rs) where all the key-value implementation is done. Here is where your data application will be managed.

## Where is my data?

The data is store inside state machines, each state machine represents a point of data and
raft enforces that all nodes have the same data in synchronization. You can have a look of
the struct [ExampleStateMachine](./src/store/mod.rs)

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
