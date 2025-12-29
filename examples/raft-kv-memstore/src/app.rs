use openraft_network_v1::ChunkedRaft;

use crate::NodeId;
use crate::StateMachineStore;
use crate::TypeConfig;

// Representation of an application state. This struct can be shared around to share
// instances of raft, store and more.
pub struct App {
    pub id: NodeId,
    pub addr: String,
    pub raft: ChunkedRaft<TypeConfig>,
    pub state_machine_store: StateMachineStore,
}
