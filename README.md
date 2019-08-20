actix-raft
==========
[![Build Status](https://travis-ci.com/railgun-rs/actix-raft.svg?branch=master)](https://travis-ci.com/railgun-rs/actix-raft)

About to hit the `0.1.0` release!

An implementation of the [Raft consensus protocol](https://raft.github.io/) using the [Actix actor framework](https://docs.rs/actix/).

This implementation differs from other Raft implementations in that:
- It is fully reactive and embraces the async ecosystem. It is driven by actual Raft related events taking place in the system as opposed to being driven by a `tick` operation. Batching of messages during replication is still used whenever possible for maximum performance.
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

### license
actix-raft is licensed under the terms of the MIT License or the Apache License 2.0, at your choosing.
