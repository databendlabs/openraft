# Distributed Key-Value Store with OpenRaft and gRPC

A distributed key-value store built using `openraft` and gRPC, demonstrating a robust, replicated storage system.

## Modules

The application is structured into key modules:

 - `src/bin`: Contains the `main()` function for server setup in [main.rs](./src/bin/main.rs)
 - `src/network`: For routing calls to their respective grpc RPCs
 - `src/grpc`:
   - `api_service.rs`: gRPC service implementations for key value store(application APIs)
   - `internal_service.rs`: Raft-specific gRPC internal network communication
   - `management_service.rs`: Administrative gRPC endpoints for cluster management
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
See the [ExampleStateMachine](./src/store/mod.rs) for implementation details.

## Cluster Management

Node management process:
- Store node information in the storage layer
- Add nodes as learners
- Promote learners to full cluster members

Note: This is an example implementation and not recommended for production use.
