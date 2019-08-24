Raft
====
The central most type of this crate is the `Raft` type.

It is a highly generic actor with the signature `Raft<D: AppData, E: AppError, N: RaftNetwork<D>, S: RaftStorage<D, E>>`. The generics here allow `Raft` to use statically known types, defined in the parent application using this crate, for maximum performance and type-safety. Users of this Raft implementation get to choose the exact types they want to use for application specific error handling coming from the storage layer, and also get to work with their application's data types directly without the overhead of serializing and deserializing the data as it moves through the `Raft` system.

### API
As the `Raft` type is an Actix [`Actor`](https://docs.rs/actix/latest/actix/trait.Actor.html), all interaction with `Raft` is handled via message passing. All pertinent message types derive the serde traits for easier integration with other data serialization formats in the Rust ecosystem, providing maximum flexibility for applications using this crate.

All message types are sent to a `Raft` node via the actor's [`Addr`](https://docs.rs/actix/latest/actix/struct.Addr.html). Applications using this crate are expected to have networking capabilities for cluster communication & client interaction. Applications are responsible for handling client requests & Raft RPCs coming from their network layer, and must send them to the `Raft` actor returning the response. More details on this topic can be found in the [network chapter](https://railgun-rs.github.io/actix-raft/network.html).

The public API of the `Raft` type is broken up into 3 sections: Client Requests, Raft RPCs & Admin Commands.

##### Client Requests
- [ClientPayload](https://docs.rs/actix-raft/latest/actix-raft/messages/struct.ClientPayload.html): a payload of data which needs to be committed to the Raft cluster. Typically, this will be data coming from application clients.

##### Raft RPCs
- [AppendEntriesRequest](https://docs.rs/actix-raft/latest/actix-raft/messages/struct.AppendEntriesRequest.html): An RPC invoked by the leader to replicate log entries (§5.3); also used as heartbeat (§5.2).
- [VoteRequest](https://docs.rs/actix-raft/latest/actix-raft/messages/struct.VoteRequest.html): An RPC invoked by candidates to gather votes (§5.2).
- [InstallSnapshotRequest](https://docs.rs/actix-raft/latest/actix-raft/messages/struct.InstallSnapshotRequest.html): Invoked by the Raft leader to send chunks of a snapshot to a follower (§7).

##### Admin Commands
- [InitWithConfig](https://docs.rs/actix-raft/latest/actix-raft/messages/struct.InitWithConfig.html): Initialize a pristine Raft node with the given config & start a campaign to become leader.
- [ProposeConfigChange](https://docs.rs/actix-raft/latest/actix-raft/messages/struct.ProposeConfigChange.html): Propose a new membership config change to a running cluster.

----

The API is simple enough, but there is more to learn about `Raft` then just feeding it messages. The next logical topic to understand is [Raft networking](https://railgun-rs.github.io/actix-raft/network.html).
