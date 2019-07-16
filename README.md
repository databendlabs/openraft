actix-raft
==========
An implementation of the [Raft consensus protocol](https://raft.github.io/) using the [Actix actor framework](https://docs.rs/actix/).

This implementation differs from other Raft implementations in that:
- It is fully reactive and embraces the async ecosystem. It is driven by actual Raft related events taking place in the system as opposed to being driven by a `tick` operation. Message buffering is still used for maximum performance.
- Storage and network integration is well defined via the two traits `RaftStorage` & `RaftNetwork`. This provides applications maximum flexibility in being able to choose their storage and networking mediums. This also allows for the storage interface to be synchronous or asynchronous based on the storage engine used, and allows for easily integrating with the actix ecosystem's networking components for efficient async networking.
- Feeding Raft requests & client requests into a running Raft node is also well defined via the Actix messages defined under the `messages` module.
- Fully supports dynamic cluster membership according to the Raft spec. See the [`ProposeConfigChange`](./docs/admin-commands.md#proposeconfigchange) admin command defined below.

This implementation strictly adheres to the [Raft spec](https://raft.github.io/raft.pdf) (*pdf warning*), and all data models use the same nomenclature found in the spec for better understandability.

### admin commands
Actix-raft nodes may be controlled in various ways outside of the normal flow of the Raft protocol using the `admin` message types. This allows the parent application — within which the Raft node is running — to influence the Raft node's behavior based on application level needs. An admin command which probably every application using this Raft implementation will need to use is `ProposeConfigChange`. See the API documentation for more details. Here are a few of the admin commands available.

See the [adminn commands docs](./docs/admin-commands.md) for more details.

### snapshots
The snapshot capabilities defined in the Raft spec are fully supported by this implementation. The storage layer is left to the application which uses this Raft implementation, but all snapshot behavior defined in the Raft spec is supported, including:

- Configurable snapshot policies. This allows nodes to perform log compacation at configurable intervals.
- Leader based `InstallSnapshot` RPC support. This allows the Raft leader to make determinations on when a new member (or a slow member) should receive a snapshot in order to come up-to-date faster.

The API documentation is comprehensive and includes implementation tips and references to the Raft spec for further guidance.
