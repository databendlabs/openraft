use openraft::Raft;

use crate::network::rpc::ExampleNetwork;
use crate::store::ExampleRequest;
use crate::store::ExampleResponse;
use crate::store::ExampleStore;

pub mod app;
pub mod network;
pub mod store;

pub type ExampleRaft = Raft<ExampleRequest, ExampleResponse, ExampleNetwork, ExampleStore>;
