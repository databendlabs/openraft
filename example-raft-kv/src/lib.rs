use openraft::Raft;

use crate::network::ExampleNetwork;
use crate::store::ExampleRequest;
use crate::store::ExampleResponse;
use crate::store::ExampleStore;

pub mod app;
pub mod network;
pub mod rpc_handlers;
pub mod store;

pub type ExampleRaft = Raft<ExampleRequest, ExampleResponse, ExampleNetwork, ExampleStore>;
