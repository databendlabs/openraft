use std::sync::Arc;

use openraft::Config;
use openraft::NodeId;

use crate::ExampleRaft;
use crate::ExampleStore;

// Representation of an application state. This struct can be shared around to share
// instances of raft, store and more.
pub struct ExampleApp {
    pub id: NodeId,
    pub raft: ExampleRaft,
    pub store: Arc<ExampleStore>,
    pub client: Arc<reqwest::Client>,
    pub config: Arc<Config>,
}
