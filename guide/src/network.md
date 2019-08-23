Network
=======
Raft is a distributed consensus protocol, so the ability to send and receive data over a network is integral to the proper functionality of nodes within a Raft cluster.

The network capabilities required by this system are broken up into two parts: the application network & the `RaftNetwork` trait.

### Application Network
When building an application around Raft, the application will often have a few common networking components:
- discovery: a component which allows the members of an application cluster (its nodes) to discover and communicate with each other. This is not provided by this crate. There are lots of solutions out there to solve this problem. Applications can build their own discovery system by way of DNS, they could use other systems like etcd or consul. The important thing to note here is that once a peer is discovered, it would be prudent for application nodes to maintain a connection with that peer, or at a minimum the address so that new connections can be made. This depeneds on the application's networking model.
- data format: the way that data is serialized and sent accross the networking medium. Popular data formats include protobuf, capnproto, flatbuffers, message pack, JSON &c. Applications are responsible for serializing and deserializing the various message types used in this crate for network transmission. Serde is used throughout this system to aid on this front.

At a minimum, applications must be able to facility message exchange between nodes reliably.

### `trait RaftNetwork`
The requirement of an application's ability to be able to send and receive Raft RPCs in defined by the `RaftNetwork` trait.

The trait is defined as:

```rust
pub trait RaftNetwork<D>
    where
        D: AppData,
        Self: Actor<Context=Context<Self>>,

        Self: Handler<AppendEntriesRequest<D>>,
        Self::Context: ToEnvelope<Self, AppendEntriesRequest<D>>,

        Self: Handler<InstallSnapshotRequest>,
        Self::Context: ToEnvelope<Self, InstallSnapshotRequest>,

        Self: Handler<VoteRequest>,
        Self::Context: ToEnvelope<Self, VoteRequest>,
{}
```

Stated simply, all this trait requires is that the implementing type be an Actix [`Actor`](https://docs.rs/actix/latest/actix) & that it implement handlers for the following message types:
- `AppendEntriesRequest`
- `InstallSnapshotRequest`
- `VoteRequest`

The type used to implement `RaftNetwork` could be the same type used to provide the other networking capabilities of an application, or it could be an independent type. The requirement is that the implementing type must be able to transmit the RPCs it receives on its handlers to the target Raft nodes identified in the RPCs. This trait is used directly by the `Raft` actor to send heartbeats to other nodes in the Raft cluster to maintain leadership, replicate entries, request votes when an election takes place, and to install snapshots.

----

Now that we've got a solid taste for the network requirements, the next logic topic to understand is [Raft storage](https://railgun-rs.github.io/actix-raft/storage.html).
