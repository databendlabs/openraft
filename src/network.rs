//! The RaftNetwork interface.

use actix::{
    dev::ToEnvelope,
    prelude::*,
};

use crate::{
    AppData,
    messages::{
        AppendEntriesRequest,
        InstallSnapshotRequest,
        VoteRequest,
    },
};

/// A trait defining the interface of a Raft network actor.
///
/// See the [network chapter of the guide](https://railgun-rs.github.io/actix-raft/network.html)
/// for details and discussion on this trait and how to implement it.
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
