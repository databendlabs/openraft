# Distributed Key-Value Store with OpenRaft and gRPC

A distributed key-value store built using `openraft` and gRPC, demonstrating a robust, replicated storage system.

This example stores Raft logs in a in-memory storage, state-machine in a
protobuf defined in-memory struct `StateMachineData`, and use gRPC for
inter-node Raft-protocol communication and the communication between app client
and the cluster.

This example provide two testing scenarios:

- `./tests/test_cluster.rs` brings up a 3 node cluster in a single process to
    prove functions, including, initialize, add-learner, change-membership and
    write/read.

- `./test-cluster.sh` provides a more realistic scenario: it brings up a 3
    process to form a cluster, and run the similar steps: initialize,
    add-learner, change-membership and write/read.

## Modules

The application is structured into key modules:

 - `src/bin`: Contains the `main()` function for server setup in [main.rs](./src/bin/main.rs)
 - `src/network`: For routing calls to their respective grpc RPCs
 - `src/grpc`:
   - `api_service.rs`: gRPC service implementations for key value store(application APIs) and managements
   - `raft_service.rs`: Raft-specific gRPC internal network communication
 - `protos`: Protocol buffers specifications for above services
 - `src/store`: Implements the key-value store logic in [store/mod.rs](./src/store/mod.rs)

## Running the Cluster


### Build the Application

```shell
cargo build
```

### Start Nodes

Start the first node:
```shell
./raft-key-value --id 1 --addr 127.0.0.1:21001
```

Start additional nodes by changing the `id` and `grpc-addr`:
```shell
./raft-key-value --id 2 --addr 127.0.0.1:21002
```

### Cluster Setup

1. Initialize the first node as the leader
2. Add learner nodes
3. Change membership to include all nodes
4. Write and read data using gRPC calls

## Data Storage

Data is stored in state machines, with Raft ensuring data synchronization across all nodes. 

## Cluster Management

Node management process:
- Store node information in the storage layer
- Add nodes as learners
- Promote learners to full cluster members

Note: This is an example implementation and not recommended for production use.
