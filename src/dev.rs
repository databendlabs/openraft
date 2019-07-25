//! Development and testing utilities.

use std::collections::BTreeMap;

use actix::prelude::*;
use crate::{
    Raft, NodeId,
    messages::{
        AppendEntriesRequest, AppendEntriesResponse,
        ClientPayload, ClientPayloadForwarded, ClientPayloadResponse, ClientError,
        InstallSnapshotRequest, InstallSnapshotResponse,
        VoteRequest, VoteResponse,
    },
    network::RaftNetwork,
    memory_storage::{MemoryStorage, MemoryStorageError},
    metrics::{RaftMetrics},
};
use log::{debug};

const ERR_ROUTING_FAILURE: &str = "Routing failures are not allowed in tests.";

/// A concrete Raft type used during testing.
pub type MemRaft = Raft<MemoryStorageError, RaftRouter, MemoryStorage>;

//////////////////////////////////////////////////////////////////////////////////////////////////
// RaftRouter ////////////////////////////////////////////////////////////////////////////////////

/// An actor which emulates a network transport and implements the `RaftNetwork` trait.
#[derive(Default)]
pub struct RaftRouter {
    pub routing_table: BTreeMap<NodeId, Addr<MemRaft>>,
    pub metrics: BTreeMap<NodeId, RaftMetrics>,
    /// Nodes which are isolated can neither send nor receive frames.
    isolated_nodes: Vec<NodeId>,
}

impl RaftRouter {
    /// Create a new instance.
    pub fn new() -> Self {
        Default::default()
    }

    /// Isolate the network of the specified node.
    pub fn isolate_node(&mut self, id: NodeId) {
        debug!("Isolating network for node {}.", &id);
        self.isolated_nodes.push(id);
    }

    /// Restore the network of the specified node.
    pub fn restore_node(&mut self, id: NodeId) {
        if let Some((idx, _)) = self.isolated_nodes.iter().enumerate().find(|(_, e)| *e == &id) {
            debug!("Restoring network for node {}.", &id);
            self.isolated_nodes.remove(idx);
        }
    }
}

impl Actor for RaftRouter {
    type Context = Context<Self>;
}

//////////////////////////////////////////////////////////////////////////////
// Impl RaftNetwork //////////////////////////////////////////////////////////

impl RaftNetwork<MemoryStorageError> for RaftRouter {}

impl Handler<AppendEntriesRequest> for RaftRouter {
    type Result = ResponseActFuture<Self, AppendEntriesResponse, ()>;

    fn handle(&mut self, msg: AppendEntriesRequest, _: &mut Self::Context) -> Self::Result {
        let addr = self.routing_table.get(&msg.target).unwrap();
        if self.isolated_nodes.contains(&msg.target) || self.isolated_nodes.contains(&msg.leader_id) {
            return Box::new(fut::err(()));
        }
        Box::new(fut::wrap_future(addr.send(msg))
            .map_err(|_, _, _| panic!(ERR_ROUTING_FAILURE))
            .and_then(|res, _, _| fut::result(res)))
    }
}

impl Handler<VoteRequest> for RaftRouter {
    type Result = ResponseActFuture<Self, VoteResponse, ()>;

    fn handle(&mut self, msg: VoteRequest, _: &mut Self::Context) -> Self::Result {
        let addr = self.routing_table.get(&msg.target).unwrap();
        if self.isolated_nodes.contains(&msg.target) || self.isolated_nodes.contains(&msg.candidate_id) {
            return Box::new(fut::err(()));
        }
        Box::new(fut::wrap_future(addr.send(msg))
            .map_err(|_, _, _| panic!(ERR_ROUTING_FAILURE))
            .and_then(|res, _, _| fut::result(res)))
    }
}

impl Handler<InstallSnapshotRequest> for RaftRouter {
    type Result = ResponseActFuture<Self, InstallSnapshotResponse, ()>;

    fn handle(&mut self, msg: InstallSnapshotRequest, _: &mut Self::Context) -> Self::Result {
        let addr = self.routing_table.get(&msg.target).unwrap();
        if self.isolated_nodes.contains(&msg.target) || self.isolated_nodes.contains(&msg.leader_id) {
            return Box::new(fut::err(()));
        }
        Box::new(fut::wrap_future(addr.send(msg))
            .map_err(|_, _, _| panic!(ERR_ROUTING_FAILURE))
            .and_then(|res, _, _| fut::result(res)))
    }
}

impl Handler<ClientPayloadForwarded<MemoryStorageError>> for RaftRouter {
    type Result = ResponseActFuture<Self, ClientPayloadResponse, ClientError<MemoryStorageError>>;

    fn handle(&mut self, msg: ClientPayloadForwarded<MemoryStorageError>, _: &mut Self::Context) -> Self::Result {
        let addr = self.routing_table.get(&msg.target).unwrap();
        if self.isolated_nodes.contains(&msg.target) || self.isolated_nodes.contains(&msg.from) {
            return Box::new(fut::err(ClientError::Internal));
        }
        Box::new(fut::wrap_future(addr.send(msg.payload))
            .map_err(|_, _, _| panic!(ERR_ROUTING_FAILURE))
            .and_then(|res, _, _| fut::result(res)))
    }
}

//////////////////////////////////////////////////////////////////////////////
// RaftMetrics ///////////////////////////////////////////////////////////////

impl Handler<RaftMetrics> for RaftRouter {
    type Result = ();

    fn handle(&mut self, msg: RaftMetrics, _: &mut Context<Self>) -> Self::Result {
        debug!("Metrics received: {:?}", &msg);
        self.metrics.insert(msg.id, msg);
    }
}

//////////////////////////////////////////////////////////////////////////////
// Test Commands /////////////////////////////////////////////////////////////

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

#[derive(Message)]
pub struct ExecuteInRaftRouter(pub Box<dyn FnOnce(&mut RaftRouter, &mut Context<RaftRouter>) + Send + 'static>);

impl Handler<ExecuteInRaftRouter> for RaftRouter {
    type Result = ();

    fn handle(&mut self, msg: ExecuteInRaftRouter, ctx: &mut Self::Context) -> Self::Result {
        msg.0(self, ctx);
    }
}


//////////////////////////////////////////////////////////////////////////////////////////////////
// RaftRecorder //////////////////////////////////////////////////////////////////////////////////

/// An actor which emulates a `RaftNetwork` but simply records requests and returns configured responses.
///
/// This type conforms to the definition of a test stub and a test spie. The responses which you
/// want to have returned for tests must be specified in order, and all requests are recorded for
/// later assertions.
///
/// For all request types, if no response is configured for a request, an error variant
/// will be returned.
#[derive(Default)]
pub struct RaftRecorder {
    metrics: Vec<RaftMetrics>,
    append_entries_requests: Vec<AppendEntriesRequest>,
    append_entries_responses: Vec<AppendEntriesResponse>,
    vote_requests: Vec<VoteRequest>,
    vote_responses: Vec<VoteResponse>,
    install_snapshot_requests: Vec<InstallSnapshotRequest>,
    install_snapshot_responses: Vec<InstallSnapshotResponse>,
    client_requests: Vec<ClientPayload<MemoryStorageError>>,
    client_responses: Vec<Result<ClientPayloadResponse, ClientError<MemoryStorageError>>>,
}

impl RaftRecorder {
    /// Create a new instance.
    pub fn new() -> Self {
        Default::default()
    }

    /// An iterator of recorded metrics events from the Raft node.
    pub fn metrics(&self) -> impl Iterator<Item=&RaftMetrics> {
        self.metrics.iter()
    }

    /// An iterator of recorded requests of this respective type.
    pub fn append_entries_requests(&self) -> impl Iterator<Item=&AppendEntriesRequest> {
        self.append_entries_requests.iter()
    }

    /// Configure the responses for append entries requests.
    pub fn append_entries_responses(&mut self, mut responses: Vec<AppendEntriesResponse>) -> &mut Self {
        responses.reverse();
        self.append_entries_responses.append(&mut responses);
        self
    }

    /// An iterator of recorded requests of this respective type.
    pub fn vote_requests(&self) -> impl Iterator<Item=&VoteRequest> {
        self.vote_requests.iter()
    }

    /// Configure the responses for vote requests.
    pub fn vote_responses(&mut self, mut responses: Vec<VoteResponse>) -> &mut Self {
        responses.reverse();
        self.vote_responses.append(&mut responses);
        self
    }

    /// An iterator of recorded requests of this respective type.
    pub fn install_snapshot_requests(&self) -> impl Iterator<Item=&InstallSnapshotRequest> {
        self.install_snapshot_requests.iter()
    }

    /// Configure the responses for install snapshot requests.
    pub fn install_snapshot_responses(&mut self, mut responses: Vec<InstallSnapshotResponse>) -> &mut Self {
        responses.reverse();
        self.install_snapshot_responses.append(&mut responses);
        self
    }

    /// An iterator of recorded requests of this respective type.
    pub fn client_requests(&self) -> impl Iterator<Item=&ClientPayload<MemoryStorageError>> {
        self.client_requests.iter()
    }

    /// Configure the responses for client requests.
    pub fn client_responses(&mut self, mut responses: Vec<Result<ClientPayloadResponse, ClientError<MemoryStorageError>>>) -> &mut Self {
        responses.reverse();
        self.client_responses.append(&mut responses);
        self
    }
}

impl Actor for RaftRecorder {
    type Context = Context<Self>;
}

//////////////////////////////////////////////////////////////////////////////
// Impl RaftNetwork //////////////////////////////////////////////////////////

impl RaftNetwork<MemoryStorageError> for RaftRecorder {}

impl Handler<AppendEntriesRequest> for RaftRecorder {
    type Result = ResponseActFuture<Self, AppendEntriesResponse, ()>;

    fn handle(&mut self, msg: AppendEntriesRequest, _: &mut Self::Context) -> Self::Result {
        self.append_entries_requests.push(msg);
        let res = self.append_entries_responses.pop().ok_or_else(|| ());
        Box::new(fut::result(res))
    }
}

impl Handler<VoteRequest> for RaftRecorder {
    type Result = ResponseActFuture<Self, VoteResponse, ()>;

    fn handle(&mut self, msg: VoteRequest, _: &mut Self::Context) -> Self::Result {
        self.vote_requests.push(msg);
        let res = self.vote_responses.pop().ok_or_else(|| ());
        Box::new(fut::result(res))
    }
}

impl Handler<InstallSnapshotRequest> for RaftRecorder {
    type Result = ResponseActFuture<Self, InstallSnapshotResponse, ()>;

    fn handle(&mut self, msg: InstallSnapshotRequest, _: &mut Self::Context) -> Self::Result {
        self.install_snapshot_requests.push(msg);
        let res = self.install_snapshot_responses.pop().ok_or_else(|| ());
        Box::new(fut::result(res))
    }
}

impl Handler<ClientPayloadForwarded<MemoryStorageError>> for RaftRecorder {
    type Result = ResponseActFuture<Self, ClientPayloadResponse, ClientError<MemoryStorageError>>;

    fn handle(&mut self, msg: ClientPayloadForwarded<MemoryStorageError>, _: &mut Self::Context) -> Self::Result {
        self.client_requests.push(msg.payload);
        let res = self.client_responses
            .pop()
            .ok_or_else(|| ClientError::Internal)
            .and_then(|res| res); // Flatten inner result.
        Box::new(fut::result(res))
    }
}

//////////////////////////////////////////////////////////////////////////////
// RaftMetrics ///////////////////////////////////////////////////////////////

impl Handler<RaftMetrics> for RaftRecorder {
    type Result = ();

    fn handle(&mut self, msg: RaftMetrics, _: &mut Context<Self>) -> Self::Result {
        self.metrics.push(msg);
    }
}

//////////////////////////////////////////////////////////////////////////////
// Test Commands /////////////////////////////////////////////////////////////

#[derive(Message)]
pub struct Assert(pub Box<dyn FnOnce(&mut RaftRecorder, &mut Context<RaftRecorder>) + Send + 'static>);

impl Handler<Assert> for RaftRecorder {
    type Result = ();

    fn handle(&mut self, msg: Assert, ctx: &mut Self::Context) -> Self::Result {
        msg.0(self, ctx);
    }
}
