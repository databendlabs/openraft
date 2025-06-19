#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("RaftCore closed receiver")]
pub(crate) struct RaftCoreClosed;
