actix-raft
==========
[![Build Status](https://travis-ci.com/railgun-rs/actix-raft.svg?branch=master)](https://travis-ci.com/railgun-rs/actix-raft)

About to hit the `0.1.0` release!

An implementation of the [Raft consensus protocol](https://raft.github.io/) using the [Actix actor framework](https://docs.rs/actix/).

This implementation differs from other Raft implementations in that:
- It is fully reactive and embraces the async ecosystem. It is driven by actual Raft related events taking place in the system as opposed to being driven by a `tick` operation. Batching of messages during replication is still used whenever possible for maximum throughput.
- Storage and network integration is well defined via the two traits `RaftStorage` & `RaftNetwork`. This provides applications maximum flexibility in being able to choose their storage and networking mediums. This also allows for the storage interface to be synchronous or asynchronous based on the storage engine used, and allows for easily integrating with the actix ecosystem's networking components for efficient async networking.
- Pumping Raft requests & client requests into a running Raft node is also well defined via the Actix messages defined under the `messages` module in this crate.
- It fully supports dynamic cluster membership changes according to the Raft spec. See the [`ProposeConfigChange`](./docs/admin-commands.md#proposeconfigchange) admin command in the docs.
- Details on initial cluster formation, and how to effectively do so from an application level perspective, are discussed in the [admin command docs](./docs/admin-commands.md).

This implementation strictly adheres to the [Raft spec](https://raft.github.io/raft.pdf) (*pdf warning*), and all data models use the same nomenclature found in the spec for better understandability. This implementation of Raft has integration tests covering all aspects of a Raft cluster's lifecycle including: cluster formation, dynamic membership changes, snapshotting, writing data to a live cluster and more.

### admin commands
Raft nodes may be controlled in various ways outside of the normal flow of the Raft protocol using the `admin` message types. This allows the parent application — within which the Raft node is running — to influence the Raft node's behavior based on application level needs. This includes the dynamic cluster membership and cluster initialization commands. See the [admin commands docs](./docs/admin-commands.md) for more details.

### data types & communication
All pertinent Raft data types in the system derive the serde traits for easier integration with other data serialization formats for maximum flexibility for applications being built on top of Raft.

### snapshots and log compaction
The snapshot and log compaction capabilities defined in the Raft spec are fully supported by this implementation. The storage layer is left to the application which uses this Raft implementation, but all snapshot behavior defined in the Raft spec is supported. Additionally, this implemention supports:

- Configurable snapshot policies. This allows nodes to perform log compacation at configurable intervals.
- Leader based `InstallSnapshot` RPC support. This allows the Raft leader to make determinations on when a new member (or a slow member) should receive a snapshot in order to come up-to-date faster.

The API documentation is comprehensive and includes implementation tips and references to the Raft spec for further guidance.

### overview
A few things to understand first. This crate's `Raft` type is an Actix actor which is intended to run within some parent application, which traditionally will be some sort of data storage system (SQL, NoSQL, KV-store, AMQP, Streaming, whatever). Inasmuch as the `Raft` instance is an actor, it is expected that the parent application is also built upon the Actix actor framework, though that is not technically required.

To use this this crate, applications must also implement this crate's `RaftStorage` & `RaftNetwork` traits. See the docs for more details on what these traits represent and how to implement them. In brief, the implementing types must be actors which can handle specific message types which correspond to everything needed for Raft storage and networking.

The following diagram shows how client requests are presented to Raft from within an application, how the data is stored, replicated and ultimately applied to the application's state machine.

<p>
    <img src="./docs/images/raft-workflow-client-requests.png"/>
</p>

The numbered elements represent segments of the workflow.
1. The parent application has received a client request, and presents the payload to `Raft` using the `ClientPayload` type.
2. `Raft` will present the payload to the `RaftStorage` impl via the `AppendLogEntry` type. This is the one location where the `RaftStorage` impl may return an application specific error. This could be for validation logic, enforcing unique indices, data/schema validation; whatever application level rules the application enforces, this is where it should enforce them. Close to the data, just before it hits the `Raft` log.
3. The `RaftStorage` impl responds to the `Raft` actor. If it is successful, go to step 4, else the error response will be sent back to the caller immediately. The error response is a statically known type coming from the parent application, and may contain whatever data needed for the application's needs.
4. `Raft` uses the `RaftNetwork` impl to communicate with the peers of the cluster.
5. `Raft` uses the `RaftNetwork` impl will replicate the entry to all other nodes in the cluster.
6. Follower nodes in the cluster respond upon successful replication.
7. Once the entry has been replicated to a majority of nodes in the cluster — "committed" in parlance of the Raft spec — the entry is ready to be applied to the application's state machine.
8. `Raft` will apply the entry to the application's state machine via the `ApplyToStateMachine` type.
9. The `RaftStorage` impl responds with a success (errors are not allowed here, it must succeed).
10. The success response is returned to the caller.

**NOTE:** this implementation of Raft offers the ability for client requests to receive a response once its entry has been committed, and before it is applied to the state machine. This is controlled by the `ResponseMode` enum type of the `ClientPayload`, which may be either `Committed` or `Applied`. Application's may use either depending on their needs.


### license
actix-raft is licensed under the terms of the MIT License or the Apache License 2.0, at your choosing.
