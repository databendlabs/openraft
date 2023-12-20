use std::sync::Arc;

use crate::LogStore;
use crate::NodeId;
use crate::Raft;
use crate::StateMachineStore;

// Representation of an application state. This struct can be shared around to share
// instances of raft, store and more.
pub struct App {
    pub id: NodeId,
    pub addr: String,
    pub raft: Raft,
    pub log_store: Arc<LogStore>,
    pub state_machine_store: Arc<StateMachineStore>,
    pub config: Arc<openraft::Config>,
}
