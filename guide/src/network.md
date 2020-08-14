Network
=======
Raft is a distributed consensus protocol, so the ability to send and receive data over a network is integral to the proper functionality of nodes within a Raft cluster.

The network capabilities required by this system are broken up into two parts: the `RaftNetwork` trait & the application network.

### `RaftNetwork`
Raft uses the `RaftNetwork` trait for sending Raft RPCs. This trait says nothing about how those requests should be received on the other end. There is a lot of flexibility with this trait. Maybe you want to use [Tonic gRPC](https://github.com/hyperium/tonic), or perhaps some other [HTTP-based protocol](https://github.com/seanmonstar/reqwest). One could use WebSockets, a raw TCP socket, UDP, HTTP3 ... in the end, this depends on the application's needs. Whichever option is chosen, the fundamental requirement is that an implementor of the `RaftNetwork` trait must be able to reliably transmit data over the network.

All of the methods to be implemented are similar in structure. Take this one for example:

```rust
    /// Send an AppendEntries RPC to the target Raft node (§5).
    async fn append_entries(&self, target: NodeId, rpc: AppendEntriesRequest<D>) -> Result<AppendEntriesResponse>;
```

The implementing type should use the given `NodeId` (just a `u64`) to identify the target Raft node to which the given `rpc` must be sent. For applications using a single Raft cluster, this is quite simple. If using a multi-Raft setup, cluster information could be embedded in the `RaftNetwork` implementing type, and network requests could be enriched with that cluster information before being transmitted over the network to ensure that the receiving server can pass the received `rpc` to the correct Raft cluster.

The excellent [`async_trait`](https://docs.rs/async-trait/) crate is re-exported by this crate to make implementation as easy as possible. Please see the documentation on how to use this macro to creating an async trait implementation.

### Application Network
The main role of the application network, in this context, is to handle RPCs from Raft peers and client requests coming from application clients, and then feed them into Raft. This is essentially the receiving end of the `RaftNetwork` trait, however this project does not enforce any specific interface on how this is to be implemented. The only requirement is that it work with the `RaftNetwork` trait implementation. There are a few other important things that it will probably need to do as well, depending on the application's needs, here are a few other common networking roles:

- **discovery:** a component which allows the members of an application cluster (its nodes) to discover and communicate with each other. This is not provided by this crate. There are lots of solutions out there to solve this problem. Applications can build their own discovery system by way of DNS, they could use other systems like etcd or consul. The important thing to note here is that once a peer is discovered, it would be prudent for application nodes to maintain a connection with that peer, as heartbeats are very regular, and building new network connections is not free.
- **data format:** the way that data is serialized and sent accross the networking medium. Popular data formats include protobuf, capnproto, flatbuffers, message pack, JSON &c. Applications are responsible for serializing and deserializing the various message types used in this crate for network transmission. Serde is used throughout this system to aid on this front.

Applications must be able to facilitate message exchange between nodes reliably.

----

Now that we've got a solid taste for the network requirements, let's move on to Raft storage.
