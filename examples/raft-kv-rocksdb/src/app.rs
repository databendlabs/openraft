use std::sync::Arc;

use openraft::Config;

use crate::ExampleRaft;
use crate::NodeId;
use crate::Store;

// Representation of an application state. This struct can be shared around to share
// instances of raft, store and more.
pub struct App {
    pub id: NodeId,
    pub api_addr: String,
    pub rcp_addr: String,
    pub raft: ExampleRaft,
    pub store: Arc<Store>,
    pub config: Arc<Config>,
}
