use crate::core::heartbeat::errors::raft_core_closed::RaftCoreClosed;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Stopped {
    #[error("HeartbeatWorkerStopped: {0}")]
    RaftCoreClosed(#[from] RaftCoreClosed),

    #[error("HeartbeatWorkerStopped: received shutdown signal")]
    ReceivedShutdown,
}
