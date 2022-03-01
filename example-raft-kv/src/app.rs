use std::sync::Arc;

use openraft::Config;

use crate::ExampleNodeId;
use crate::ExampleRaft;
use crate::ExampleStore;

// Representation of an application state. This struct can be shared around to share
// instances of raft, store and more.
pub struct ExampleApp {
    pub id: ExampleNodeId,
    pub addr: String,
    pub raft: ExampleRaft,
    pub store: Arc<ExampleStore>,
    pub config: Arc<Config>,
}
