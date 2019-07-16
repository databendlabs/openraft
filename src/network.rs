//! The RaftNetwork interface.

use actix::{
    dev::ToEnvelope,
    prelude::*,
};

use crate::{
    AppError,
    messages::{
        AppendEntriesRequest,
        ClientPayload,
        InstallSnapshotRequest,
        VoteRequest,
    }
};

/// A trait defining the interface of a Raft network actor.
///
/// This trait models an application's networking layer simply in terms of its ability to handle
/// the various message types of this trait. The actual transport technology being used, the data
/// serialization, all of that is up to the application which is using this crate. The only
/// requirement is that an actor implement this trait and uphold the expected invariants.
///
/// ### expectations
/// It is expected that the implementor of this trait has the actual ability to take the received
/// messages, serialize them in whatever way the application needs, and then send them by whatever
/// means to the `target` Raft node specified within the message. As follows, it is expected that
/// the application will also have the ability to receive such messages from peer Raft nodes,
/// deserialize them into concrete instances of their data type, and then feed them to the Raft
/// actor running within that process. Doing so will return the result which must be sent back
/// in response to the sender.
///
/// At the end of the day, any old RPC/HTTP networking stack will do just fine. There is a lot of
/// flexibility here. To provide the maximum level of flexibility for data serialization, all
/// types derive serde's `Serialize` & `DeserializeOwned` traits. Using other serialization
/// schemes like protobuf, capnproto or flatbuffers is simple to manage and can be done entirly
/// using the Rust's standard `From/Into` traits.
pub trait RaftNetwork<E>
    where
        E: AppError,
        Self: Actor<Context=Context<Self>>,

        Self: Handler<AppendEntriesRequest>,
        Self::Context: ToEnvelope<Self, AppendEntriesRequest>,

        Self: Handler<InstallSnapshotRequest>,
        Self::Context: ToEnvelope<Self, InstallSnapshotRequest>,

        Self: Handler<VoteRequest>,
        Self::Context: ToEnvelope<Self, VoteRequest>,

        Self: Handler<ClientPayload<E>>,
        Self::Context: ToEnvelope<Self, ClientPayload<E>>,
{}
