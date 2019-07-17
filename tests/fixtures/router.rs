use std::collections::BTreeMap;

use actix::prelude::*;
use actix_raft::{
    NodeId,
    messages::{
        AppendEntriesRequest, AppendEntriesResponse,
        ClientPayload, ClientPayloadResponse, ClientError,
        InstallSnapshotRequest, InstallSnapshotResponse,
        VoteRequest, VoteResponse,
    },
    network::RaftNetwork,
    memory_storage::{MemoryStorageError},
};

use crate::fixtures::MemRaft;

const ERR_ROUTING_FAILURE: &str = "Routing failures are not allowed in tests.";

/// An actor which emulates a network transport and implements the `RaftNetwork` trait.
pub struct RaftRouter {
    routing_table: BTreeMap<NodeId, Addr<MemRaft>>,
}

impl RaftRouter {
    /// Create a new instance.
    pub fn new() -> Self {
        Self{routing_table: Default::default()}
    }
}

impl Actor for RaftRouter {
    type Context = Context<Self>;
}

impl RaftNetwork<MemoryStorageError> for RaftRouter {}

impl Handler<AppendEntriesRequest> for RaftRouter {
    type Result = ResponseActFuture<Self, AppendEntriesResponse, ()>;

    fn handle(&mut self, msg: AppendEntriesRequest, _: &mut Self::Context) -> Self::Result {
        let addr = self.routing_table.get(&msg.target).unwrap();
        Box::new(fut::wrap_future(addr.send(msg))
            .map_err(|_, _, _| panic!(ERR_ROUTING_FAILURE))
            .and_then(|res, _, _| fut::result(res)))
    }
}

impl Handler<VoteRequest> for RaftRouter {
    type Result = ResponseActFuture<Self, VoteResponse, ()>;

    fn handle(&mut self, msg: VoteRequest, _: &mut Self::Context) -> Self::Result {
        let addr = self.routing_table.get(&msg.target).unwrap();
        Box::new(fut::wrap_future(addr.send(msg))
            .map_err(|_, _, _| panic!(ERR_ROUTING_FAILURE))
            .and_then(|res, _, _| fut::result(res)))
    }
}

impl Handler<InstallSnapshotRequest> for RaftRouter {
    type Result = ResponseActFuture<Self, InstallSnapshotResponse, ()>;

    fn handle(&mut self, msg: InstallSnapshotRequest, _: &mut Self::Context) -> Self::Result {
        let addr = self.routing_table.get(&msg.target).unwrap();
        Box::new(fut::wrap_future(addr.send(msg))
            .map_err(|_, _, _| panic!(ERR_ROUTING_FAILURE))
            .and_then(|res, _, _| fut::result(res)))
    }
}

impl Handler<ClientPayload<MemoryStorageError>> for RaftRouter {
    type Result = ResponseActFuture<Self, ClientPayloadResponse, ClientError<MemoryStorageError>>;

    fn handle(&mut self, msg: ClientPayload<MemoryStorageError>, _: &mut Self::Context) -> Self::Result {
        let addr = self.routing_table.get(&msg.target).unwrap();
        Box::new(fut::wrap_future(addr.send(msg))
            .map_err(|_, _, _| panic!(ERR_ROUTING_FAILURE))
            .and_then(|res, _, _| fut::result(res)))
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// Test Commands /////////////////////////////////////////////////////////////////////////////////

#[derive(Message)]
pub struct Register {
    pub id: NodeId,
    pub addr: Addr<MemRaft>,
}

impl Handler<Register> for RaftRouter {
    type Result = ();

    fn handle(&mut self, msg: Register, _: &mut Self::Context) -> Self::Result {
        self.routing_table.insert(msg.id, msg.addr);
    }
}
