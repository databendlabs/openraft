use crate::fixtures::MemLogStore;
use crate::fixtures::MemRaft;
use crate::fixtures::MemStateMachine;

/// A node's handle and stores, retained across an explicit stop or restart.
#[derive(Clone)]
pub struct Node {
    pub raft: MemRaft,
    pub log_store: MemLogStore,
    pub state_machine: MemStateMachine,
}
