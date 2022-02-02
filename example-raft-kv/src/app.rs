use std::sync::Arc;

use openraft::Config;
use openraft::NodeId;

use crate::ExampleRaft;
use crate::ExampleStore;

pub struct ExampleApp {
    pub id: NodeId,
    pub raft: ExampleRaft,
    pub store: Arc<ExampleStore>,
    pub config: Arc<Config>,
}
